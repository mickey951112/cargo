use cargo_platform::Cfg;
use std::collections::hash_map::{Entry, HashMap};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;

use crate::core::compiler::job_queue::JobState;
use crate::core::PackageId;
use crate::util::errors::{CargoResult, CargoResultExt};
use crate::util::machine_message::{self, Message};
use crate::util::{self, internal, paths, profile};

use super::job::{Freshness, Job, Work};
use super::{fingerprint, Context, Kind, Unit};

/// Contains the parsed output of a custom build script.
#[derive(Clone, Debug, Hash)]
pub struct BuildOutput {
    /// Paths to pass to rustc with the `-L` flag.
    pub library_paths: Vec<PathBuf>,
    /// Names and link kinds of libraries, suitable for the `-l` flag.
    pub library_links: Vec<String>,
    /// Linker arguments suitable to be passed to `-C link-arg=<args>`
    pub linker_args: Vec<String>,
    /// Various `--cfg` flags to pass to the compiler.
    pub cfgs: Vec<String>,
    /// Additional environment variables to run the compiler with.
    pub env: Vec<(String, String)>,
    /// Metadata to pass to the immediate dependencies.
    pub metadata: Vec<(String, String)>,
    /// Paths to trigger a rerun of this build script.
    /// May be absolute or relative paths (relative to package root).
    pub rerun_if_changed: Vec<PathBuf>,
    /// Environment variables which, when changed, will cause a rebuild.
    pub rerun_if_env_changed: Vec<String>,
    /// Warnings generated by this build.
    pub warnings: Vec<String>,
}

/// Map of packages to build script output.
///
/// This initially starts out as empty. Overridden build scripts get
/// inserted during `build_map`. The rest of the entries are added
/// immediately after each build script runs.
pub type BuildScriptOutputs = HashMap<(PackageId, Kind), BuildOutput>;

/// Linking information for a `Unit`.
///
/// See `build_map` for more details.
#[derive(Default)]
pub struct BuildScripts {
    /// Cargo will use this `to_link` vector to add `-L` flags to compiles as we
    /// propagate them upwards towards the final build. Note, however, that we
    /// need to preserve the ordering of `to_link` to be topologically sorted.
    /// This will ensure that build scripts which print their paths properly will
    /// correctly pick up the files they generated (if there are duplicates
    /// elsewhere).
    ///
    /// To preserve this ordering, the (id, kind) is stored in two places, once
    /// in the `Vec` and once in `seen_to_link` for a fast lookup. We maintain
    /// this as we're building interactively below to ensure that the memory
    /// usage here doesn't blow up too much.
    ///
    /// For more information, see #2354.
    pub to_link: Vec<(PackageId, Kind)>,
    /// This is only used while constructing `to_link` to avoid duplicates.
    seen_to_link: HashSet<(PackageId, Kind)>,
    /// Host-only dependencies that have build scripts.
    ///
    /// This is the set of transitive dependencies that are host-only
    /// (proc-macro, plugin, build-dependency) that contain a build script.
    /// Any `BuildOutput::library_paths` path relative to `target` will be
    /// added to LD_LIBRARY_PATH so that the compiler can find any dynamic
    /// libraries a build script may have generated.
    pub plugins: BTreeSet<PackageId>,
}

/// Dependency information as declared by a build script.
#[derive(Debug)]
pub struct BuildDeps {
    /// Absolute path to the file in the target directory that stores the
    /// output of the build script.
    pub build_script_output: PathBuf,
    /// Files that trigger a rebuild if they change.
    pub rerun_if_changed: Vec<PathBuf>,
    /// Environment variables that trigger a rebuild if they change.
    pub rerun_if_env_changed: Vec<String>,
}

/// Prepares a `Work` that executes the target as a custom build script.
pub fn prepare<'a, 'cfg>(cx: &mut Context<'a, 'cfg>, unit: &Unit<'a>) -> CargoResult<Job> {
    let _p = profile::start(format!(
        "build script prepare: {}/{}",
        unit.pkg,
        unit.target.name()
    ));

    let key = (unit.pkg.package_id(), unit.kind);

    if cx.build_script_outputs.lock().unwrap().contains_key(&key) {
        // The output is already set, thus the build script is overridden.
        fingerprint::prepare_target(cx, unit, false)
    } else {
        build_work(cx, unit)
    }
}

fn emit_build_output(state: &JobState<'_>, output: &BuildOutput, package_id: PackageId) {
    let library_paths = output
        .library_paths
        .iter()
        .map(|l| l.display().to_string())
        .collect::<Vec<_>>();

    let msg = machine_message::BuildScript {
        package_id,
        linked_libs: &output.library_links,
        linked_paths: &library_paths,
        cfgs: &output.cfgs,
        env: &output.env,
    }
    .to_json_string();
    state.stdout(msg);
}

fn build_work<'a, 'cfg>(cx: &mut Context<'a, 'cfg>, unit: &Unit<'a>) -> CargoResult<Job> {
    assert!(unit.mode.is_run_custom_build());
    let bcx = &cx.bcx;
    let dependencies = cx.dep_targets(unit);
    let build_script_unit = dependencies
        .iter()
        .find(|d| !d.mode.is_run_custom_build() && d.target.is_custom_build())
        .expect("running a script not depending on an actual script");
    let script_dir = cx.files().build_script_dir(build_script_unit);
    let script_out_dir = cx.files().build_script_out_dir(unit);
    let script_run_dir = cx.files().build_script_run_dir(unit);
    let build_plan = bcx.build_config.build_plan;
    let invocation_name = unit.buildkey();

    if let Some(deps) = unit.pkg.manifest().metabuild() {
        prepare_metabuild(cx, build_script_unit, deps)?;
    }

    // Building the command to execute
    let to_exec = script_dir.join(unit.target.name());

    // Start preparing the process to execute, starting out with some
    // environment variables. Note that the profile-related environment
    // variables are not set with this the build script's profile but rather the
    // package's library profile.
    // NOTE: if you add any profile flags, be sure to update
    // `Profiles::get_profile_run_custom_build` so that those flags get
    // carried over.
    let to_exec = to_exec.into_os_string();
    let mut cmd = cx.compilation.host_process(to_exec, unit.pkg)?;
    let debug = unit.profile.debuginfo.unwrap_or(0) != 0;
    cmd.env("OUT_DIR", &script_out_dir)
        .env("CARGO_MANIFEST_DIR", unit.pkg.root())
        .env("NUM_JOBS", &bcx.jobs().to_string())
        .env(
            "TARGET",
            &match unit.kind {
                Kind::Host => bcx.host_triple(),
                Kind::Target => bcx.target_triple(),
            },
        )
        .env("DEBUG", debug.to_string())
        .env("OPT_LEVEL", &unit.profile.opt_level.to_string())
        .env(
            "PROFILE",
            if bcx.build_config.release {
                "release"
            } else {
                "debug"
            },
        )
        .env("HOST", &bcx.host_triple())
        .env("RUSTC", &bcx.rustc.path)
        .env("RUSTDOC", &*bcx.config.rustdoc()?)
        .inherit_jobserver(&cx.jobserver);

    if let Some(ref linker) = bcx.target_config.linker {
        cmd.env("RUSTC_LINKER", linker);
    }

    if let Some(links) = unit.pkg.manifest().links() {
        cmd.env("CARGO_MANIFEST_LINKS", links);
    }

    // Be sure to pass along all enabled features for this package, this is the
    // last piece of statically known information that we have.
    for feat in &unit.features {
        cmd.env(&format!("CARGO_FEATURE_{}", super::envify(feat)), "1");
    }

    let mut cfg_map = HashMap::new();
    for cfg in bcx.cfg(unit.kind) {
        match *cfg {
            Cfg::Name(ref n) => {
                cfg_map.insert(n.clone(), None);
            }
            Cfg::KeyPair(ref k, ref v) => {
                if let Some(ref mut values) =
                    *cfg_map.entry(k.clone()).or_insert_with(|| Some(Vec::new()))
                {
                    values.push(v.clone())
                }
            }
        }
    }
    for (k, v) in cfg_map {
        let k = format!("CARGO_CFG_{}", super::envify(&k));
        match v {
            Some(list) => {
                cmd.env(&k, list.join(","));
            }
            None => {
                cmd.env(&k, "");
            }
        }
    }

    // Gather the set of native dependencies that this package has along with
    // some other variables to close over.
    //
    // This information will be used at build-time later on to figure out which
    // sorts of variables need to be discovered at that time.
    let lib_deps = {
        dependencies
            .iter()
            .filter_map(|unit| {
                if unit.mode.is_run_custom_build() {
                    Some((
                        unit.pkg.manifest().links().unwrap().to_string(),
                        unit.pkg.package_id(),
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let pkg_name = unit.pkg.to_string();
    let build_script_outputs = Arc::clone(&cx.build_script_outputs);
    let id = unit.pkg.package_id();
    let output_file = script_run_dir.join("output");
    let err_file = script_run_dir.join("stderr");
    let root_output_file = script_run_dir.join("root-output");
    let host_target_root = cx.files().host_root().to_path_buf();
    let all = (
        id,
        pkg_name.clone(),
        Arc::clone(&build_script_outputs),
        output_file.clone(),
        script_out_dir.clone(),
    );
    let build_scripts = cx.build_scripts.get(unit).cloned();
    let kind = unit.kind;
    let json_messages = bcx.build_config.emit_json();
    let extra_verbose = bcx.config.extra_verbose();
    let (prev_output, prev_script_out_dir) = prev_build_output(cx, unit);

    paths::create_dir_all(&script_dir)?;
    paths::create_dir_all(&script_out_dir)?;

    // Prepare the unit of "dirty work" which will actually run the custom build
    // command.
    //
    // Note that this has to do some extra work just before running the command
    // to determine extra environment variables and such.
    let dirty = Work::new(move |state| {
        // Make sure that OUT_DIR exists.
        //
        // If we have an old build directory, then just move it into place,
        // otherwise create it!
        paths::create_dir_all(&script_out_dir).chain_err(|| {
            internal(
                "failed to create script output directory for \
                 build command",
            )
        })?;

        // For all our native lib dependencies, pick up their metadata to pass
        // along to this custom build command. We're also careful to augment our
        // dynamic library search path in case the build script depended on any
        // native dynamic libraries.
        if !build_plan {
            let build_script_outputs = build_script_outputs.lock().unwrap();
            for (name, id) in lib_deps {
                let key = (id, kind);
                let script_output = build_script_outputs.get(&key).ok_or_else(|| {
                    internal(format!(
                        "failed to locate build state for env \
                         vars: {}/{:?}",
                        id, kind
                    ))
                })?;
                let data = &script_output.metadata;
                for &(ref key, ref value) in data.iter() {
                    cmd.env(
                        &format!("DEP_{}_{}", super::envify(&name), super::envify(key)),
                        value,
                    );
                }
            }
            if let Some(build_scripts) = build_scripts {
                super::add_plugin_deps(
                    &mut cmd,
                    &build_script_outputs,
                    &build_scripts,
                    &host_target_root,
                )?;
            }
        }

        if build_plan {
            state.build_plan(invocation_name, cmd.clone(), Arc::new(Vec::new()));
            return Ok(());
        }

        // And now finally, run the build command itself!
        state.running(&cmd);
        let timestamp = paths::set_invocation_time(&script_run_dir)?;
        let prefix = format!("[{} {}] ", id.name(), id.version());
        let output = cmd
            .exec_with_streaming(
                &mut |stdout| {
                    if extra_verbose {
                        state.stdout(format!("{}{}", prefix, stdout));
                    }
                    Ok(())
                },
                &mut |stderr| {
                    if extra_verbose {
                        state.stderr(format!("{}{}", prefix, stderr));
                    }
                    Ok(())
                },
                true,
            )
            .chain_err(|| format!("failed to run custom build command for `{}`", pkg_name))?;

        // After the build command has finished running, we need to be sure to
        // remember all of its output so we can later discover precisely what it
        // was, even if we don't run the build command again (due to freshness).
        //
        // This is also the location where we provide feedback into the build
        // state informing what variables were discovered via our script as
        // well.
        paths::write(&output_file, &output.stdout)?;
        filetime::set_file_times(output_file, timestamp, timestamp)?;
        paths::write(&err_file, &output.stderr)?;
        paths::write(&root_output_file, util::path2bytes(&script_out_dir)?)?;
        let parsed_output =
            BuildOutput::parse(&output.stdout, &pkg_name, &script_out_dir, &script_out_dir)?;

        if json_messages {
            emit_build_output(state, &parsed_output, id);
        }
        build_script_outputs
            .lock()
            .unwrap()
            .insert((id, kind), parsed_output);
        Ok(())
    });

    // Now that we've prepared our work-to-do, we need to prepare the fresh work
    // itself to run when we actually end up just discarding what we calculated
    // above.
    let fresh = Work::new(move |state| {
        let (id, pkg_name, build_script_outputs, output_file, script_out_dir) = all;
        let output = match prev_output {
            Some(output) => output,
            None => BuildOutput::parse_file(
                &output_file,
                &pkg_name,
                &prev_script_out_dir,
                &script_out_dir,
            )?,
        };

        if json_messages {
            emit_build_output(state, &output, id);
        }

        build_script_outputs
            .lock()
            .unwrap()
            .insert((id, kind), output);
        Ok(())
    });

    let mut job = if cx.bcx.build_config.build_plan {
        Job::new(Work::noop(), Freshness::Dirty)
    } else {
        fingerprint::prepare_target(cx, unit, false)?
    };
    if job.freshness() == Freshness::Dirty {
        job.before(dirty);
    } else {
        job.before(fresh);
    }
    Ok(job)
}

impl BuildOutput {
    pub fn parse_file(
        path: &Path,
        pkg_name: &str,
        script_out_dir_when_generated: &Path,
        script_out_dir: &Path,
    ) -> CargoResult<BuildOutput> {
        let contents = paths::read_bytes(path)?;
        BuildOutput::parse(
            &contents,
            pkg_name,
            script_out_dir_when_generated,
            script_out_dir,
        )
    }

    // Parses the output of a script.
    // The `pkg_name` is used for error messages.
    pub fn parse(
        input: &[u8],
        pkg_name: &str,
        script_out_dir_when_generated: &Path,
        script_out_dir: &Path,
    ) -> CargoResult<BuildOutput> {
        let mut library_paths = Vec::new();
        let mut library_links = Vec::new();
        let mut linker_args = Vec::new();
        let mut cfgs = Vec::new();
        let mut env = Vec::new();
        let mut metadata = Vec::new();
        let mut rerun_if_changed = Vec::new();
        let mut rerun_if_env_changed = Vec::new();
        let mut warnings = Vec::new();
        let whence = format!("build script of `{}`", pkg_name);

        for line in input.split(|b| *b == b'\n') {
            let line = match str::from_utf8(line) {
                Ok(line) => line.trim(),
                Err(..) => continue,
            };
            let mut iter = line.splitn(2, ':');
            if iter.next() != Some("cargo") {
                // skip this line since it doesn't start with "cargo:"
                continue;
            }
            let data = match iter.next() {
                Some(val) => val,
                None => continue,
            };

            // getting the `key=value` part of the line
            let mut iter = data.splitn(2, '=');
            let key = iter.next();
            let value = iter.next();
            let (key, value) = match (key, value) {
                (Some(a), Some(b)) => (a, b.trim_end()),
                // Line started with `cargo:` but didn't match `key=value`.
                _ => failure::bail!("Wrong output in {}: `{}`", whence, line),
            };

            // This will rewrite paths if the target directory has been moved.
            let value = value.replace(
                script_out_dir_when_generated.to_str().unwrap(),
                script_out_dir.to_str().unwrap(),
            );

            // Keep in sync with TargetConfig::new.
            match key {
                "rustc-flags" => {
                    let (paths, links) = BuildOutput::parse_rustc_flags(&value, &whence)?;
                    library_links.extend(links.into_iter());
                    library_paths.extend(paths.into_iter());
                }
                "rustc-link-lib" => library_links.push(value.to_string()),
                "rustc-link-search" => library_paths.push(PathBuf::from(value)),
                "rustc-cdylib-link-arg" => linker_args.push(value.to_string()),
                "rustc-cfg" => cfgs.push(value.to_string()),
                "rustc-env" => env.push(BuildOutput::parse_rustc_env(&value, &whence)?),
                "warning" => warnings.push(value.to_string()),
                "rerun-if-changed" => rerun_if_changed.push(PathBuf::from(value)),
                "rerun-if-env-changed" => rerun_if_env_changed.push(value.to_string()),
                _ => metadata.push((key.to_string(), value.to_string())),
            }
        }

        Ok(BuildOutput {
            library_paths,
            library_links,
            linker_args,
            cfgs,
            env,
            metadata,
            rerun_if_changed,
            rerun_if_env_changed,
            warnings,
        })
    }

    pub fn parse_rustc_flags(
        value: &str,
        whence: &str,
    ) -> CargoResult<(Vec<PathBuf>, Vec<String>)> {
        let value = value.trim();
        let mut flags_iter = value
            .split(|c: char| c.is_whitespace())
            .filter(|w| w.chars().any(|c| !c.is_whitespace()));
        let (mut library_paths, mut library_links) = (Vec::new(), Vec::new());

        while let Some(flag) = flags_iter.next() {
            if flag.starts_with("-l") || flag.starts_with("-L") {
                // Check if this flag has no space before the value as is
                // common with tools like pkg-config
                // e.g. -L/some/dir/local/lib or -licui18n
                let (flag, mut value) = flag.split_at(2);
                if value.len() == 0 {
                    value = match flags_iter.next() {
                        Some(v) => v,
                        None => failure::bail! {
                            "Flag in rustc-flags has no value in {}: {}",
                            whence,
                            value
                        },
                    }
                }

                match flag {
                    "-l" => library_links.push(value.to_string()),
                    "-L" => library_paths.push(PathBuf::from(value)),

                    // This was already checked above
                    _ => unreachable!(),
                };
            } else {
                failure::bail!(
                    "Only `-l` and `-L` flags are allowed in {}: `{}`",
                    whence,
                    value
                )
            }
        }
        Ok((library_paths, library_links))
    }

    pub fn parse_rustc_env(value: &str, whence: &str) -> CargoResult<(String, String)> {
        let mut iter = value.splitn(2, '=');
        let name = iter.next();
        let val = iter.next();
        match (name, val) {
            (Some(n), Some(v)) => Ok((n.to_owned(), v.to_owned())),
            _ => failure::bail!("Variable rustc-env has no value in {}: {}", whence, value),
        }
    }
}

fn prepare_metabuild<'a, 'cfg>(
    cx: &Context<'a, 'cfg>,
    unit: &Unit<'a>,
    deps: &[String],
) -> CargoResult<()> {
    let mut output = Vec::new();
    let available_deps = cx.dep_targets(unit);
    // Filter out optional dependencies, and look up the actual lib name.
    let meta_deps: Vec<_> = deps
        .iter()
        .filter_map(|name| {
            available_deps
                .iter()
                .find(|u| u.pkg.name().as_str() == name.as_str())
                .map(|dep| dep.target.crate_name())
        })
        .collect();
    for dep in &meta_deps {
        output.push(format!("use {};\n", dep));
    }
    output.push("fn main() {\n".to_string());
    for dep in &meta_deps {
        output.push(format!("    {}::metabuild();\n", dep));
    }
    output.push("}\n".to_string());
    let output = output.join("");
    let path = unit.pkg.manifest().metabuild_path(cx.bcx.ws.target_dir());
    paths::create_dir_all(path.parent().unwrap())?;
    paths::write_if_changed(path, &output)?;
    Ok(())
}

impl BuildDeps {
    pub fn new(output_file: &Path, output: Option<&BuildOutput>) -> BuildDeps {
        BuildDeps {
            build_script_output: output_file.to_path_buf(),
            rerun_if_changed: output
                .map(|p| &p.rerun_if_changed)
                .cloned()
                .unwrap_or_default(),
            rerun_if_env_changed: output
                .map(|p| &p.rerun_if_env_changed)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

/// Computes several maps in `Context`:
/// - `build_scripts`: A map that tracks which build scripts each package
///   depends on.
/// - `build_explicit_deps`: Dependency statements emitted by build scripts
///   from a previous run.
/// - `build_script_outputs`: Pre-populates this with any overridden build
///   scripts.
///
/// The important one here is `build_scripts`, which for each `(package,
/// kind)` stores a `BuildScripts` object which contains a list of
/// dependencies with build scripts that the unit should consider when
/// linking. For example this lists all dependencies' `-L` flags which need to
/// be propagated transitively.
///
/// The given set of units to this function is the initial set of
/// targets/profiles which are being built.
pub fn build_map<'b, 'cfg>(cx: &mut Context<'b, 'cfg>, units: &[Unit<'b>]) -> CargoResult<()> {
    let mut ret = HashMap::new();
    for unit in units {
        build(&mut ret, cx, unit)?;
    }
    cx.build_scripts
        .extend(ret.into_iter().map(|(k, v)| (k, Arc::new(v))));
    return Ok(());

    // Recursive function to build up the map we're constructing. This function
    // memoizes all of its return values as it goes along.
    fn build<'a, 'b, 'cfg>(
        out: &'a mut HashMap<Unit<'b>, BuildScripts>,
        cx: &mut Context<'b, 'cfg>,
        unit: &Unit<'b>,
    ) -> CargoResult<&'a BuildScripts> {
        // Do a quick pre-flight check to see if we've already calculated the
        // set of dependencies.
        if out.contains_key(unit) {
            return Ok(&out[unit]);
        }

        // If there is a build script override, pre-fill the build output.
        if let Some(links) = unit.pkg.manifest().links() {
            if let Some(output) = cx.bcx.script_override(links, unit.kind) {
                let key = (unit.pkg.package_id(), unit.kind);
                cx.build_script_outputs
                    .lock()
                    .unwrap()
                    .insert(key, output.clone());
            }
        }

        let mut ret = BuildScripts::default();

        if !unit.target.is_custom_build() && unit.pkg.has_custom_build() {
            add_to_link(&mut ret, unit.pkg.package_id(), unit.kind);
        }

        // Load any dependency declarations from a previous run.
        if unit.mode.is_run_custom_build() {
            parse_previous_explicit_deps(cx, unit)?;
        }

        // We want to invoke the compiler deterministically to be cache-friendly
        // to rustc invocation caching schemes, so be sure to generate the same
        // set of build script dependency orderings via sorting the targets that
        // come out of the `Context`.
        let mut dependencies = cx.dep_targets(unit);
        dependencies.sort_by_key(|u| u.pkg.package_id());

        for dep_unit in dependencies.iter() {
            let dep_scripts = build(out, cx, dep_unit)?;

            if dep_unit.target.for_host() {
                ret.plugins
                    .extend(dep_scripts.to_link.iter().map(|p| &p.0).cloned());
            } else if dep_unit.target.linkable() {
                for &(pkg, kind) in dep_scripts.to_link.iter() {
                    add_to_link(&mut ret, pkg, kind);
                }
            }
        }

        match out.entry(*unit) {
            Entry::Vacant(entry) => Ok(entry.insert(ret)),
            Entry::Occupied(_) => panic!("cyclic dependencies in `build_map`"),
        }
    }

    // When adding an entry to 'to_link' we only actually push it on if the
    // script hasn't seen it yet (e.g., we don't push on duplicates).
    fn add_to_link(scripts: &mut BuildScripts, pkg: PackageId, kind: Kind) {
        if scripts.seen_to_link.insert((pkg, kind)) {
            scripts.to_link.push((pkg, kind));
        }
    }

    fn parse_previous_explicit_deps<'a, 'cfg>(
        cx: &mut Context<'a, 'cfg>,
        unit: &Unit<'a>,
    ) -> CargoResult<()> {
        let script_run_dir = cx.files().build_script_run_dir(unit);
        let output_file = script_run_dir.join("output");
        let (prev_output, _) = prev_build_output(cx, unit);
        let deps = BuildDeps::new(&output_file, prev_output.as_ref());
        cx.build_explicit_deps.insert(*unit, deps);
        Ok(())
    }
}

/// Returns the previous parsed `BuildOutput`, if any, from a previous
/// execution.
///
/// Also returns the directory containing the output, typically used later in
/// processing.
fn prev_build_output<'a, 'cfg>(
    cx: &mut Context<'a, 'cfg>,
    unit: &Unit<'a>,
) -> (Option<BuildOutput>, PathBuf) {
    let script_out_dir = cx.files().build_script_out_dir(unit);
    let script_run_dir = cx.files().build_script_run_dir(unit);
    let root_output_file = script_run_dir.join("root-output");
    let output_file = script_run_dir.join("output");

    let prev_script_out_dir = paths::read_bytes(&root_output_file)
        .and_then(|bytes| util::bytes2path(&bytes))
        .unwrap_or_else(|_| script_out_dir.clone());

    (
        BuildOutput::parse_file(
            &output_file,
            &unit.pkg.to_string(),
            &prev_script_out_dir,
            &script_out_dir,
        )
        .ok(),
        prev_script_out_dir,
    )
}
