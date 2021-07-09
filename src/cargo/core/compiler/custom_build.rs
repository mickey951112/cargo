use super::job::{Freshness, Job, Work};
use super::{fingerprint, Context, LinkType, Unit};
use crate::core::compiler::context::Metadata;
use crate::core::compiler::job_queue::JobState;
use crate::core::{profiles::ProfileRoot, PackageId, Target};
use crate::util::errors::CargoResult;
use crate::util::machine_message::{self, Message};
use crate::util::{internal, profile};
use anyhow::{bail, Context as _};
use cargo_platform::Cfg;
use cargo_util::paths;
use std::collections::hash_map::{Entry, HashMap};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::{Arc, Mutex};

const CARGO_WARNING: &str = "cargo:warning=";

/// Contains the parsed output of a custom build script.
#[derive(Clone, Debug, Hash, Default)]
pub struct BuildOutput {
    /// Paths to pass to rustc with the `-L` flag.
    pub library_paths: Vec<PathBuf>,
    /// Names and link kinds of libraries, suitable for the `-l` flag.
    pub library_links: Vec<String>,
    /// Linker arguments suitable to be passed to `-C link-arg=<args>`
    pub linker_args: Vec<(LinkType, String)>,
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
    ///
    /// These are only displayed if this is a "local" package, `-vv` is used,
    /// or there is a build error for any target in this package.
    pub warnings: Vec<String>,
}

/// Map of packages to build script output.
///
/// This initially starts out as empty. Overridden build scripts get
/// inserted during `build_map`. The rest of the entries are added
/// immediately after each build script runs.
///
/// The `Metadata` is the unique metadata hash for the RunCustomBuild Unit of
/// the package. It needs a unique key, since the build script can be run
/// multiple times with different profiles or features. We can't embed a
/// `Unit` because this structure needs to be shareable between threads.
#[derive(Default)]
pub struct BuildScriptOutputs {
    outputs: HashMap<Metadata, BuildOutput>,
}

/// Linking information for a `Unit`.
///
/// See `build_map` for more details.
#[derive(Default)]
pub struct BuildScripts {
    /// List of build script outputs this Unit needs to include for linking. Each
    /// element is an index into `BuildScriptOutputs`.
    ///
    /// Cargo will use this `to_link` vector to add `-L` flags to compiles as we
    /// propagate them upwards towards the final build. Note, however, that we
    /// need to preserve the ordering of `to_link` to be topologically sorted.
    /// This will ensure that build scripts which print their paths properly will
    /// correctly pick up the files they generated (if there are duplicates
    /// elsewhere).
    ///
    /// To preserve this ordering, the (id, metadata) is stored in two places, once
    /// in the `Vec` and once in `seen_to_link` for a fast lookup. We maintain
    /// this as we're building interactively below to ensure that the memory
    /// usage here doesn't blow up too much.
    ///
    /// For more information, see #2354.
    pub to_link: Vec<(PackageId, Metadata)>,
    /// This is only used while constructing `to_link` to avoid duplicates.
    seen_to_link: HashSet<(PackageId, Metadata)>,
    /// Host-only dependencies that have build scripts. Each element is an
    /// index into `BuildScriptOutputs`.
    ///
    /// This is the set of transitive dependencies that are host-only
    /// (proc-macro, plugin, build-dependency) that contain a build script.
    /// Any `BuildOutput::library_paths` path relative to `target` will be
    /// added to LD_LIBRARY_PATH so that the compiler can find any dynamic
    /// libraries a build script may have generated.
    pub plugins: BTreeSet<(PackageId, Metadata)>,
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
pub fn prepare(cx: &mut Context<'_, '_>, unit: &Unit) -> CargoResult<Job> {
    let _p = profile::start(format!(
        "build script prepare: {}/{}",
        unit.pkg,
        unit.target.name()
    ));

    let metadata = cx.get_run_build_script_metadata(unit);
    if cx
        .build_script_outputs
        .lock()
        .unwrap()
        .contains_key(metadata)
    {
        // The output is already set, thus the build script is overridden.
        fingerprint::prepare_target(cx, unit, false)
    } else {
        build_work(cx, unit)
    }
}

fn emit_build_output(
    state: &JobState<'_, '_>,
    output: &BuildOutput,
    out_dir: &Path,
    package_id: PackageId,
) -> CargoResult<()> {
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
        out_dir,
    }
    .to_json_string();
    state.stdout(msg)?;
    Ok(())
}

fn build_work(cx: &mut Context<'_, '_>, unit: &Unit) -> CargoResult<Job> {
    assert!(unit.mode.is_run_custom_build());
    let bcx = &cx.bcx;
    let dependencies = cx.unit_deps(unit);
    let build_script_unit = dependencies
        .iter()
        .find(|d| !d.unit.mode.is_run_custom_build() && d.unit.target.is_custom_build())
        .map(|d| &d.unit)
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
    let mut cmd = cx.compilation.host_process(to_exec, &unit.pkg)?;
    let debug = unit.profile.debuginfo.unwrap_or(0) != 0;
    cmd.env("OUT_DIR", &script_out_dir)
        .env("CARGO_MANIFEST_DIR", unit.pkg.root())
        .env("NUM_JOBS", &bcx.jobs().to_string())
        .env("TARGET", bcx.target_data.short_name(&unit.kind))
        .env("DEBUG", debug.to_string())
        .env("OPT_LEVEL", &unit.profile.opt_level.to_string())
        .env(
            "PROFILE",
            match unit.profile.root {
                ProfileRoot::Release => "release",
                ProfileRoot::Debug => "debug",
            },
        )
        .env("HOST", &bcx.host_triple())
        .env("RUSTC", &bcx.rustc().path)
        .env("RUSTDOC", &*bcx.config.rustdoc()?)
        .inherit_jobserver(&cx.jobserver);

    if let Some(linker) = &bcx.target_data.target_config(unit.kind).linker {
        cmd.env(
            "RUSTC_LINKER",
            linker.val.clone().resolve_program(bcx.config),
        );
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
    for cfg in bcx.target_data.cfg(unit.kind) {
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
        if k == "debug_assertions" {
            // This cfg is always true and misleading, so avoid setting it.
            // That is because Cargo queries rustc without any profile settings.
            continue;
        }
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
    let lib_deps = dependencies
        .iter()
        .filter_map(|dep| {
            if dep.unit.mode.is_run_custom_build() {
                let dep_metadata = cx.get_run_build_script_metadata(&dep.unit);
                Some((
                    dep.unit.pkg.manifest().links().unwrap().to_string(),
                    dep.unit.pkg.package_id(),
                    dep_metadata,
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let library_name = unit.pkg.library().map(|t| t.crate_name());
    let pkg_descr = unit.pkg.to_string();
    let build_script_outputs = Arc::clone(&cx.build_script_outputs);
    let id = unit.pkg.package_id();
    let output_file = script_run_dir.join("output");
    let err_file = script_run_dir.join("stderr");
    let root_output_file = script_run_dir.join("root-output");
    let host_target_root = cx.files().host_dest().to_path_buf();
    let all = (
        id,
        library_name.clone(),
        pkg_descr.clone(),
        Arc::clone(&build_script_outputs),
        output_file.clone(),
        script_out_dir.clone(),
    );
    let build_scripts = cx.build_scripts.get(unit).cloned();
    let json_messages = bcx.build_config.emit_json();
    let extra_verbose = bcx.config.extra_verbose();
    let (prev_output, prev_script_out_dir) = prev_build_output(cx, unit);
    let metadata_hash = cx.get_run_build_script_metadata(unit);

    paths::create_dir_all(&script_dir)?;
    paths::create_dir_all(&script_out_dir)?;

    let extra_link_arg = cx.bcx.config.cli_unstable().extra_link_arg;
    let nightly_features_allowed = cx.bcx.config.nightly_features_allowed;
    let targets: Vec<Target> = unit.pkg.targets().to_vec();
    // Need a separate copy for the fresh closure.
    let targets_fresh = targets.clone();

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
        paths::create_dir_all(&script_out_dir)
            .with_context(|| "failed to create script output directory for build command")?;

        // For all our native lib dependencies, pick up their metadata to pass
        // along to this custom build command. We're also careful to augment our
        // dynamic library search path in case the build script depended on any
        // native dynamic libraries.
        if !build_plan {
            let build_script_outputs = build_script_outputs.lock().unwrap();
            for (name, dep_id, dep_metadata) in lib_deps {
                let script_output = build_script_outputs.get(dep_metadata).ok_or_else(|| {
                    internal(format!(
                        "failed to locate build state for env vars: {}/{}",
                        dep_id, dep_metadata
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
        let mut warnings_in_case_of_panic = Vec::new();
        let output = cmd
            .exec_with_streaming(
                &mut |stdout| {
                    if let Some(warning) = stdout.strip_prefix(CARGO_WARNING) {
                        warnings_in_case_of_panic.push(warning.to_owned());
                    }
                    if extra_verbose {
                        state.stdout(format!("{}{}", prefix, stdout))?;
                    }
                    Ok(())
                },
                &mut |stderr| {
                    if extra_verbose {
                        state.stderr(format!("{}{}", prefix, stderr))?;
                    }
                    Ok(())
                },
                true,
            )
            .with_context(|| format!("failed to run custom build command for `{}`", pkg_descr));

        if let Err(error) = output {
            insert_warnings_in_build_outputs(
                build_script_outputs,
                id,
                metadata_hash,
                warnings_in_case_of_panic,
            );
            return Err(error);
        }

        let output = output.unwrap();

        // After the build command has finished running, we need to be sure to
        // remember all of its output so we can later discover precisely what it
        // was, even if we don't run the build command again (due to freshness).
        //
        // This is also the location where we provide feedback into the build
        // state informing what variables were discovered via our script as
        // well.
        paths::write(&output_file, &output.stdout)?;
        // This mtime shift allows Cargo to detect if a source file was
        // modified in the middle of the build.
        paths::set_file_time_no_err(output_file, timestamp);
        paths::write(&err_file, &output.stderr)?;
        paths::write(&root_output_file, paths::path2bytes(&script_out_dir)?)?;
        let parsed_output = BuildOutput::parse(
            &output.stdout,
            library_name,
            &pkg_descr,
            &script_out_dir,
            &script_out_dir,
            extra_link_arg,
            nightly_features_allowed,
            &targets,
        )?;

        if json_messages {
            emit_build_output(state, &parsed_output, script_out_dir.as_path(), id)?;
        }
        build_script_outputs
            .lock()
            .unwrap()
            .insert(id, metadata_hash, parsed_output);
        Ok(())
    });

    // Now that we've prepared our work-to-do, we need to prepare the fresh work
    // itself to run when we actually end up just discarding what we calculated
    // above.
    let fresh = Work::new(move |state| {
        let (id, library_name, pkg_descr, build_script_outputs, output_file, script_out_dir) = all;
        let output = match prev_output {
            Some(output) => output,
            None => BuildOutput::parse_file(
                &output_file,
                library_name,
                &pkg_descr,
                &prev_script_out_dir,
                &script_out_dir,
                extra_link_arg,
                nightly_features_allowed,
                &targets_fresh,
            )?,
        };

        if json_messages {
            emit_build_output(state, &output, script_out_dir.as_path(), id)?;
        }

        build_script_outputs
            .lock()
            .unwrap()
            .insert(id, metadata_hash, output);
        Ok(())
    });

    let mut job = if cx.bcx.build_config.build_plan {
        Job::new_dirty(Work::noop())
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

fn insert_warnings_in_build_outputs(
    build_script_outputs: Arc<Mutex<BuildScriptOutputs>>,
    id: PackageId,
    metadata_hash: Metadata,
    warnings: Vec<String>,
) {
    let build_output_with_only_warnings = BuildOutput {
        warnings,
        ..BuildOutput::default()
    };
    build_script_outputs
        .lock()
        .unwrap()
        .insert(id, metadata_hash, build_output_with_only_warnings);
}

impl BuildOutput {
    pub fn parse_file(
        path: &Path,
        library_name: Option<String>,
        pkg_descr: &str,
        script_out_dir_when_generated: &Path,
        script_out_dir: &Path,
        extra_link_arg: bool,
        nightly_features_allowed: bool,
        targets: &[Target],
    ) -> CargoResult<BuildOutput> {
        let contents = paths::read_bytes(path)?;
        BuildOutput::parse(
            &contents,
            library_name,
            pkg_descr,
            script_out_dir_when_generated,
            script_out_dir,
            extra_link_arg,
            nightly_features_allowed,
            targets,
        )
    }

    // Parses the output of a script.
    // The `pkg_descr` is used for error messages.
    // The `library_name` is used for determining if RUSTC_BOOTSTRAP should be allowed.
    pub fn parse(
        input: &[u8],
        // Takes String instead of InternedString so passing `unit.pkg.name()` will give a compile error.
        library_name: Option<String>,
        pkg_descr: &str,
        script_out_dir_when_generated: &Path,
        script_out_dir: &Path,
        extra_link_arg: bool,
        nightly_features_allowed: bool,
        targets: &[Target],
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
        let whence = format!("build script of `{}`", pkg_descr);

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
                _ => bail!("Wrong output in {}: `{}`", whence, line),
            };

            // This will rewrite paths if the target directory has been moved.
            let value = value.replace(
                script_out_dir_when_generated.to_str().unwrap(),
                script_out_dir.to_str().unwrap(),
            );

            // Keep in sync with TargetConfig::parse_links_overrides.
            match key {
                "rustc-flags" => {
                    let (paths, links) = BuildOutput::parse_rustc_flags(&value, &whence)?;
                    library_links.extend(links.into_iter());
                    library_paths.extend(paths.into_iter());
                }
                "rustc-link-lib" => library_links.push(value.to_string()),
                "rustc-link-search" => library_paths.push(PathBuf::from(value)),
                "rustc-link-arg-cdylib" | "rustc-cdylib-link-arg" => {
                    if !targets.iter().any(|target| target.is_cdylib()) {
                        warnings.push(format!(
                            "cargo:{} was specified in the build script of {}, \
                             but that package does not contain a cdylib target\n\
                             \n\
                             Allowing this was an unintended change in the 1.50 \
                             release, and may become an error in the future. \
                             For more information, see \
                             <https://github.com/rust-lang/cargo/issues/9562>.",
                            key, pkg_descr
                        ));
                    }
                    linker_args.push((LinkType::Cdylib, value))
                }
                "rustc-link-arg-bins" => {
                    if extra_link_arg {
                        if !targets.iter().any(|target| target.is_bin()) {
                            bail!(
                                "invalid instruction `cargo:{}` from {}\n\
                                 The package {} does not have a bin target.",
                                key,
                                whence,
                                pkg_descr
                            );
                        }
                        linker_args.push((LinkType::Bin, value));
                    } else {
                        warnings.push(format!("cargo:{} requires -Zextra-link-arg flag", key));
                    }
                }
                "rustc-link-arg-bin" => {
                    if extra_link_arg {
                        let mut parts = value.splitn(2, '=');
                        let bin_name = parts.next().unwrap().to_string();
                        let arg = parts.next().ok_or_else(|| {
                            anyhow::format_err!(
                                "invalid instruction `cargo:{}={}` from {}\n\
                                 The instruction should have the form cargo:{}=BIN=ARG",
                                key,
                                value,
                                whence,
                                key
                            )
                        })?;
                        if !targets
                            .iter()
                            .any(|target| target.is_bin() && target.name() == bin_name)
                        {
                            bail!(
                                "invalid instruction `cargo:{}` from {}\n\
                                 The package {} does not have a bin target with the name `{}`.",
                                key,
                                whence,
                                pkg_descr,
                                bin_name
                            );
                        }
                        linker_args.push((LinkType::SingleBin(bin_name), arg.to_string()));
                    } else {
                        warnings.push(format!("cargo:{} requires -Zextra-link-arg flag", key));
                    }
                }
                "rustc-link-arg" => {
                    if extra_link_arg {
                        linker_args.push((LinkType::All, value));
                    } else {
                        warnings.push(format!("cargo:{} requires -Zextra-link-arg flag", key));
                    }
                }
                "rustc-cfg" => cfgs.push(value.to_string()),
                "rustc-env" => {
                    let (key, val) = BuildOutput::parse_rustc_env(&value, &whence)?;
                    // Build scripts aren't allowed to set RUSTC_BOOTSTRAP.
                    // See https://github.com/rust-lang/cargo/issues/7088.
                    if key == "RUSTC_BOOTSTRAP" {
                        // If RUSTC_BOOTSTRAP is already set, the user of Cargo knows about
                        // bootstrap and still wants to override the channel. Give them a way to do
                        // so, but still emit a warning that the current crate shouldn't be trying
                        // to set RUSTC_BOOTSTRAP.
                        // If this is a nightly build, setting RUSTC_BOOTSTRAP wouldn't affect the
                        // behavior, so still only give a warning.
                        // NOTE: cargo only allows nightly features on RUSTC_BOOTSTRAP=1, but we
                        // want setting any value of RUSTC_BOOTSTRAP to downgrade this to a warning
                        // (so that `RUSTC_BOOTSTRAP=library_name` will work)
                        let rustc_bootstrap_allows = |name: Option<&str>| {
                            let name = match name {
                                // as of 2021, no binaries on crates.io use RUSTC_BOOTSTRAP, so
                                // fine-grained opt-outs aren't needed. end-users can always use
                                // RUSTC_BOOTSTRAP=1 from the top-level if it's really a problem.
                                None => return false,
                                Some(n) => n,
                            };
                            std::env::var("RUSTC_BOOTSTRAP")
                                .map_or(false, |var| var.split(',').any(|s| s == name))
                        };
                        if nightly_features_allowed
                            || rustc_bootstrap_allows(library_name.as_deref())
                        {
                            warnings.push(format!("Cannot set `RUSTC_BOOTSTRAP={}` from {}.\n\
                                note: Crates cannot set `RUSTC_BOOTSTRAP` themselves, as doing so would subvert the stability guarantees of Rust for your project.",
                                val, whence
                            ));
                        } else {
                            // Setting RUSTC_BOOTSTRAP would change the behavior of the crate.
                            // Abort with an error.
                            bail!("Cannot set `RUSTC_BOOTSTRAP={}` from {}.\n\
                                note: Crates cannot set `RUSTC_BOOTSTRAP` themselves, as doing so would subvert the stability guarantees of Rust for your project.\n\
                                help: If you're sure you want to do this in your project, set the environment variable `RUSTC_BOOTSTRAP={}` before running cargo instead.",
                                val,
                                whence,
                                library_name.as_deref().unwrap_or("1"),
                            );
                        }
                    } else {
                        env.push((key, val));
                    }
                }
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
                if value.is_empty() {
                    value = match flags_iter.next() {
                        Some(v) => v,
                        None => bail! {
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
                bail!(
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
            _ => bail!("Variable rustc-env has no value in {}: {}", whence, value),
        }
    }
}

fn prepare_metabuild(cx: &Context<'_, '_>, unit: &Unit, deps: &[String]) -> CargoResult<()> {
    let mut output = Vec::new();
    let available_deps = cx.unit_deps(unit);
    // Filter out optional dependencies, and look up the actual lib name.
    let meta_deps: Vec<_> = deps
        .iter()
        .filter_map(|name| {
            available_deps
                .iter()
                .find(|d| d.unit.pkg.name().as_str() == name.as_str())
                .map(|d| d.unit.target.crate_name())
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
/// metadata)` stores a `BuildScripts` object which contains a list of
/// dependencies with build scripts that the unit should consider when
/// linking. For example this lists all dependencies' `-L` flags which need to
/// be propagated transitively.
///
/// The given set of units to this function is the initial set of
/// targets/profiles which are being built.
pub fn build_map(cx: &mut Context<'_, '_>) -> CargoResult<()> {
    let mut ret = HashMap::new();
    for unit in &cx.bcx.roots {
        build(&mut ret, cx, unit)?;
    }
    cx.build_scripts
        .extend(ret.into_iter().map(|(k, v)| (k, Arc::new(v))));
    return Ok(());

    // Recursive function to build up the map we're constructing. This function
    // memoizes all of its return values as it goes along.
    fn build<'a>(
        out: &'a mut HashMap<Unit, BuildScripts>,
        cx: &mut Context<'_, '_>,
        unit: &Unit,
    ) -> CargoResult<&'a BuildScripts> {
        // Do a quick pre-flight check to see if we've already calculated the
        // set of dependencies.
        if out.contains_key(unit) {
            return Ok(&out[unit]);
        }

        // If there is a build script override, pre-fill the build output.
        if unit.mode.is_run_custom_build() {
            if let Some(links) = unit.pkg.manifest().links() {
                if let Some(output) = cx.bcx.target_data.script_override(links, unit.kind) {
                    let metadata = cx.get_run_build_script_metadata(unit);
                    cx.build_script_outputs.lock().unwrap().insert(
                        unit.pkg.package_id(),
                        metadata,
                        output.clone(),
                    );
                }
            }
        }

        let mut ret = BuildScripts::default();

        // If a package has a build script, add itself as something to inspect for linking.
        if !unit.target.is_custom_build() && unit.pkg.has_custom_build() {
            let script_meta = cx
                .find_build_script_metadata(unit)
                .expect("has_custom_build should have RunCustomBuild");
            add_to_link(&mut ret, unit.pkg.package_id(), script_meta);
        }

        // Load any dependency declarations from a previous run.
        if unit.mode.is_run_custom_build() {
            parse_previous_explicit_deps(cx, unit);
        }

        // We want to invoke the compiler deterministically to be cache-friendly
        // to rustc invocation caching schemes, so be sure to generate the same
        // set of build script dependency orderings via sorting the targets that
        // come out of the `Context`.
        let mut dependencies: Vec<Unit> =
            cx.unit_deps(unit).iter().map(|d| d.unit.clone()).collect();
        dependencies.sort_by_key(|u| u.pkg.package_id());

        for dep_unit in dependencies.iter() {
            let dep_scripts = build(out, cx, dep_unit)?;

            if dep_unit.target.for_host() {
                ret.plugins.extend(dep_scripts.to_link.iter().cloned());
            } else if dep_unit.target.is_linkable() {
                for &(pkg, metadata) in dep_scripts.to_link.iter() {
                    add_to_link(&mut ret, pkg, metadata);
                }
            }
        }

        match out.entry(unit.clone()) {
            Entry::Vacant(entry) => Ok(entry.insert(ret)),
            Entry::Occupied(_) => panic!("cyclic dependencies in `build_map`"),
        }
    }

    // When adding an entry to 'to_link' we only actually push it on if the
    // script hasn't seen it yet (e.g., we don't push on duplicates).
    fn add_to_link(scripts: &mut BuildScripts, pkg: PackageId, metadata: Metadata) {
        if scripts.seen_to_link.insert((pkg, metadata)) {
            scripts.to_link.push((pkg, metadata));
        }
    }

    fn parse_previous_explicit_deps(cx: &mut Context<'_, '_>, unit: &Unit) {
        let script_run_dir = cx.files().build_script_run_dir(unit);
        let output_file = script_run_dir.join("output");
        let (prev_output, _) = prev_build_output(cx, unit);
        let deps = BuildDeps::new(&output_file, prev_output.as_ref());
        cx.build_explicit_deps.insert(unit.clone(), deps);
    }
}

/// Returns the previous parsed `BuildOutput`, if any, from a previous
/// execution.
///
/// Also returns the directory containing the output, typically used later in
/// processing.
fn prev_build_output(cx: &mut Context<'_, '_>, unit: &Unit) -> (Option<BuildOutput>, PathBuf) {
    let script_out_dir = cx.files().build_script_out_dir(unit);
    let script_run_dir = cx.files().build_script_run_dir(unit);
    let root_output_file = script_run_dir.join("root-output");
    let output_file = script_run_dir.join("output");

    let prev_script_out_dir = paths::read_bytes(&root_output_file)
        .and_then(|bytes| paths::bytes2path(&bytes))
        .unwrap_or_else(|_| script_out_dir.clone());

    let extra_link_arg = cx.bcx.config.cli_unstable().extra_link_arg;

    (
        BuildOutput::parse_file(
            &output_file,
            unit.pkg.library().map(|t| t.crate_name()),
            &unit.pkg.to_string(),
            &prev_script_out_dir,
            &script_out_dir,
            extra_link_arg,
            cx.bcx.config.nightly_features_allowed,
            unit.pkg.targets(),
        )
        .ok(),
        prev_script_out_dir,
    )
}

impl BuildScriptOutputs {
    /// Inserts a new entry into the map.
    fn insert(&mut self, pkg_id: PackageId, metadata: Metadata, parsed_output: BuildOutput) {
        match self.outputs.entry(metadata) {
            Entry::Vacant(entry) => {
                entry.insert(parsed_output);
            }
            Entry::Occupied(entry) => panic!(
                "build script output collision for {}/{}\n\
                old={:?}\nnew={:?}",
                pkg_id,
                metadata,
                entry.get(),
                parsed_output
            ),
        }
    }

    /// Returns `true` if the given key already exists.
    fn contains_key(&self, metadata: Metadata) -> bool {
        self.outputs.contains_key(&metadata)
    }

    /// Gets the build output for the given key.
    pub fn get(&self, meta: Metadata) -> Option<&BuildOutput> {
        self.outputs.get(&meta)
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&Metadata, &BuildOutput)> {
        self.outputs.iter()
    }
}
