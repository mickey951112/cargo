use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{self, Path, PathBuf};
use std::sync::Arc;

use same_file::is_same_file;
use serde_json;

use core::profiles::{Lto, Profile};
use core::shell::ColorChoice;
use core::{PackageId, Target};
use util::errors::{CargoResult, CargoResultExt, Internal};
use util::paths;
use util::{self, machine_message, Freshness, ProcessBuilder};
use util::{internal, join_paths, profile};

use self::build_plan::BuildPlan;
use self::job::{Job, Work};
use self::job_queue::JobQueue;

use self::output_depinfo::output_depinfo;

pub use self::build_context::{BuildContext, FileFlavor, TargetConfig, TargetInfo};
pub use self::build_config::{BuildConfig, CompileMode, MessageFormat};
pub use self::compilation::Compilation;
pub use self::context::{Context, Unit};
pub use self::custom_build::{BuildMap, BuildOutput, BuildScripts};
pub use self::layout::is_bad_artifact_name;

mod build_config;
mod build_context;
mod build_plan;
mod compilation;
mod context;
mod custom_build;
mod fingerprint;
mod job;
mod job_queue;
mod layout;
mod output_depinfo;

/// Whether an object is for the host arch, or the target arch.
///
/// These will be the same unless cross-compiling.
#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy, PartialOrd, Ord, Serialize)]
pub enum Kind {
    Host,
    Target,
}

/// A glorified callback for executing calls to rustc. Rather than calling rustc
/// directly, we'll use an Executor, giving clients an opportunity to intercept
/// the build calls.
pub trait Executor: Send + Sync + 'static {
    /// Called after a rustc process invocation is prepared up-front for a given
    /// unit of work (may still be modified for runtime-known dependencies, when
    /// the work is actually executed).
    fn init(&self, _cx: &Context, _unit: &Unit) {}

    /// In case of an `Err`, Cargo will not continue with the build process for
    /// this package.
    fn exec(&self, cmd: ProcessBuilder, _id: &PackageId, _target: &Target) -> CargoResult<()> {
        cmd.exec()?;
        Ok(())
    }

    fn exec_json(
        &self,
        cmd: ProcessBuilder,
        _id: &PackageId,
        _target: &Target,
        handle_stdout: &mut FnMut(&str) -> CargoResult<()>,
        handle_stderr: &mut FnMut(&str) -> CargoResult<()>,
    ) -> CargoResult<()> {
        cmd.exec_with_streaming(handle_stdout, handle_stderr, false)?;
        Ok(())
    }

    /// Queried when queuing each unit of work. If it returns true, then the
    /// unit will always be rebuilt, independent of whether it needs to be.
    fn force_rebuild(&self, _unit: &Unit) -> bool {
        false
    }
}

/// A `DefaultExecutor` calls rustc without doing anything else. It is Cargo's
/// default behaviour.
#[derive(Copy, Clone)]
pub struct DefaultExecutor;

impl Executor for DefaultExecutor {}

fn compile<'a, 'cfg: 'a>(
    cx: &mut Context<'a, 'cfg>,
    jobs: &mut JobQueue<'a>,
    plan: &mut BuildPlan,
    unit: &Unit<'a>,
    exec: &Arc<Executor>,
) -> CargoResult<()> {
    let bcx = cx.bcx;
    let build_plan = bcx.build_config.build_plan;
    if !cx.compiled.insert(*unit) {
        return Ok(());
    }

    // Build up the work to be done to compile this unit, enqueuing it once
    // we've got everything constructed.
    let p = profile::start(format!("preparing: {}/{}", unit.pkg, unit.target.name()));
    fingerprint::prepare_init(cx, unit)?;
    cx.links.validate(bcx.resolve, unit)?;

    let (dirty, fresh, freshness) = if unit.mode.is_run_custom_build() {
        custom_build::prepare(cx, unit)?
    } else if unit.mode == CompileMode::Doctest {
        // we run these targets later, so this is just a noop for now
        (Work::noop(), Work::noop(), Freshness::Fresh)
    } else if build_plan {
        (
            rustc(cx, unit, &exec.clone())?,
            Work::noop(),
            Freshness::Dirty,
        )
    } else {
        let (mut freshness, dirty, fresh) = fingerprint::prepare_target(cx, unit)?;
        let work = if unit.mode.is_doc() {
            rustdoc(cx, unit)?
        } else {
            rustc(cx, unit, exec)?
        };
        // Need to link targets on both the dirty and fresh
        let dirty = work.then(link_targets(cx, unit, false)?).then(dirty);
        let fresh = link_targets(cx, unit, true)?.then(fresh);

        if exec.force_rebuild(unit) {
            freshness = Freshness::Dirty;
        }

        (dirty, fresh, freshness)
    };
    jobs.enqueue(cx, unit, Job::new(dirty, fresh), freshness)?;
    drop(p);

    // Be sure to compile all dependencies of this target as well.
    for unit in cx.dep_targets(unit).iter() {
        compile(cx, jobs, plan, unit, exec)?;
    }
    if build_plan {
        plan.add(cx, unit)?;
    }

    Ok(())
}

fn rustc<'a, 'cfg>(
    mut cx: &mut Context<'a, 'cfg>,
    unit: &Unit<'a>,
    exec: &Arc<Executor>,
) -> CargoResult<Work> {
    let mut rustc = prepare_rustc(cx, &unit.target.rustc_crate_types(), unit)?;
    let build_plan = cx.bcx.build_config.build_plan;

    let name = unit.pkg.name().to_string();
    let buildkey = unit.buildkey();

    // If this is an upstream dep we don't want warnings from, turn off all
    // lints.
    if !cx.bcx.show_warnings(unit.pkg.package_id()) {
        rustc.arg("--cap-lints").arg("allow");

    // If this is an upstream dep but we *do* want warnings, make sure that they
    // don't fail compilation.
    } else if !unit.pkg.package_id().source_id().is_path() {
        rustc.arg("--cap-lints").arg("warn");
    }

    let outputs = cx.outputs(unit)?;
    let root = cx.files().out_dir(unit);
    let kind = unit.kind;

    // Prepare the native lib state (extra -L and -l flags)
    let build_state = cx.build_state.clone();
    let current_id = unit.pkg.package_id().clone();
    let build_deps = load_build_deps(cx, unit);

    // If we are a binary and the package also contains a library, then we
    // don't pass the `-l` flags.
    let pass_l_flag = unit.target.is_lib() || !unit.pkg.targets().iter().any(|t| t.is_lib());
    let do_rename = unit.target.allows_underscores() && !unit.mode.is_any_test();
    let real_name = unit.target.name().to_string();
    let crate_name = unit.target.crate_name();

    // XXX(Rely on target_filenames iterator as source of truth rather than rederiving filestem)
    let rustc_dep_info_loc = if do_rename && cx.files().metadata(unit).is_none() {
        root.join(&crate_name)
    } else {
        root.join(&cx.files().file_stem(unit))
    }.with_extension("d");
    let dep_info_loc = fingerprint::dep_info_loc(&mut cx, unit);

    rustc.args(&cx.bcx.rustflags_args(unit)?);
    let json_messages = cx.bcx.build_config.json_messages();
    let package_id = unit.pkg.package_id().clone();
    let target = unit.target.clone();

    exec.init(cx, unit);
    let exec = exec.clone();

    let root_output = cx.files().target_root().to_path_buf();
    let pkg_root = unit.pkg.root().to_path_buf();
    let cwd = rustc
        .get_cwd()
        .unwrap_or_else(|| cx.bcx.config.cwd())
        .to_path_buf();

    return Ok(Work::new(move |state| {
        // Only at runtime have we discovered what the extra -L and -l
        // arguments are for native libraries, so we process those here. We
        // also need to be sure to add any -L paths for our plugins to the
        // dynamic library load path as a plugin's dynamic library may be
        // located somewhere in there.
        // Finally, if custom environment variables have been produced by
        // previous build scripts, we include them in the rustc invocation.
        if let Some(build_deps) = build_deps {
            let build_state = build_state.outputs.lock().unwrap();
            if !build_plan {
                add_native_deps(
                    &mut rustc,
                    &build_state,
                    &build_deps,
                    pass_l_flag,
                    &current_id,
                )?;
                add_plugin_deps(&mut rustc, &build_state, &build_deps, &root_output)?;
            }
            add_custom_env(&mut rustc, &build_state, &current_id, kind)?;
        }

        for output in outputs.iter() {
            // If there is both an rmeta and rlib, rustc will prefer to use the
            // rlib, even if it is older. Therefore, we must delete the rlib to
            // force using the new rmeta.
            if output.path.extension() == Some(OsStr::new("rmeta")) {
                let dst = root.join(&output.path).with_extension("rlib");
                if dst.exists() {
                    paths::remove_file(&dst)?;
                }
            }
        }

        state.running(&rustc);
        if json_messages {
            exec.exec_json(
                rustc,
                &package_id,
                &target,
                &mut |line| {
                    if !line.is_empty() {
                        Err(internal(&format!(
                            "compiler stdout is not empty: `{}`",
                            line
                        )))
                    } else {
                        Ok(())
                    }
                },
                &mut |line| {
                    // stderr from rustc can have a mix of JSON and non-JSON output
                    if line.starts_with('{') {
                        // Handle JSON lines
                        let compiler_message = serde_json::from_str(line).map_err(|_| {
                            internal(&format!("compiler produced invalid json: `{}`", line))
                        })?;

                        machine_message::emit(&machine_message::FromCompiler {
                            package_id: &package_id,
                            target: &target,
                            message: compiler_message,
                        });
                    } else {
                        // Forward non-JSON to stderr
                        writeln!(io::stderr(), "{}", line)?;
                    }
                    Ok(())
                },
            ).chain_err(|| format!("Could not compile `{}`.", name))?;
        } else if build_plan {
            state.build_plan(buildkey, rustc.clone(), outputs.clone());
        } else {
            exec.exec(rustc, &package_id, &target)
                .map_err(Internal::new)
                .chain_err(|| format!("Could not compile `{}`.", name))?;
        }

        if do_rename && real_name != crate_name {
            let dst = &outputs[0].path;
            let src = dst.with_file_name(
                dst.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace(&real_name, &crate_name),
            );
            if src.exists() && src.file_name() != dst.file_name() {
                fs::rename(&src, &dst)
                    .chain_err(|| internal(format!("could not rename crate {:?}", src)))?;
            }
        }

        if rustc_dep_info_loc.exists() {
            fingerprint::translate_dep_info(&rustc_dep_info_loc, &dep_info_loc, &pkg_root, &cwd)
                .chain_err(|| {
                    internal(format!(
                        "could not parse/generate dep info at: {}",
                        rustc_dep_info_loc.display()
                    ))
                })?;
        }

        Ok(())
    }));

    // Add all relevant -L and -l flags from dependencies (now calculated and
    // present in `state`) to the command provided
    fn add_native_deps(
        rustc: &mut ProcessBuilder,
        build_state: &BuildMap,
        build_scripts: &BuildScripts,
        pass_l_flag: bool,
        current_id: &PackageId,
    ) -> CargoResult<()> {
        for key in build_scripts.to_link.iter() {
            let output = build_state.get(key).ok_or_else(|| {
                internal(format!(
                    "couldn't find build state for {}/{:?}",
                    key.0, key.1
                ))
            })?;
            for path in output.library_paths.iter() {
                rustc.arg("-L").arg(path);
            }
            if key.0 == *current_id {
                for cfg in &output.cfgs {
                    rustc.arg("--cfg").arg(cfg);
                }
                if pass_l_flag {
                    for name in output.library_links.iter() {
                        rustc.arg("-l").arg(name);
                    }
                }
            }
        }
        Ok(())
    }

    // Add all custom environment variables present in `state` (after they've
    // been put there by one of the `build_scripts`) to the command provided.
    fn add_custom_env(
        rustc: &mut ProcessBuilder,
        build_state: &BuildMap,
        current_id: &PackageId,
        kind: Kind,
    ) -> CargoResult<()> {
        let key = (current_id.clone(), kind);
        if let Some(output) = build_state.get(&key) {
            for &(ref name, ref value) in output.env.iter() {
                rustc.env(name, value);
            }
        }
        Ok(())
    }
}

/// Link the compiled target (often of form `foo-{metadata_hash}`) to the
/// final target. This must happen during both "Fresh" and "Compile"
fn link_targets<'a, 'cfg>(
    cx: &mut Context<'a, 'cfg>,
    unit: &Unit<'a>,
    fresh: bool,
) -> CargoResult<Work> {
    let bcx = cx.bcx;
    let outputs = cx.outputs(unit)?;
    let export_dir = cx.files().export_dir();
    let package_id = unit.pkg.package_id().clone();
    let target = unit.target.clone();
    let profile = unit.profile;
    let unit_mode = unit.mode;
    let features = bcx.resolve
        .features_sorted(&package_id)
        .into_iter()
        .map(|s| s.to_owned())
        .collect();
    let json_messages = bcx.build_config.json_messages();

    Ok(Work::new(move |_| {
        // If we're a "root crate", e.g. the target of this compilation, then we
        // hard link our outputs out of the `deps` directory into the directory
        // above. This means that `cargo build` will produce binaries in
        // `target/debug` which one probably expects.
        let mut destinations = vec![];
        for output in outputs.iter() {
            let src = &output.path;
            // This may have been a `cargo rustc` command which changes the
            // output, so the source may not actually exist.
            if !src.exists() {
                continue;
            }
            let dst = match output.hardlink.as_ref() {
                Some(dst) => dst,
                None => {
                    destinations.push(src.display().to_string());
                    continue;
                }
            };
            destinations.push(dst.display().to_string());
            hardlink_or_copy(src, dst)?;
            if let Some(ref path) = export_dir {
                if !path.exists() {
                    fs::create_dir_all(path)?;
                }

                hardlink_or_copy(src, &path.join(dst.file_name().unwrap()))?;
            }
        }

        if json_messages {
            let art_profile = machine_message::ArtifactProfile {
                opt_level: profile.opt_level.as_str(),
                debuginfo: profile.debuginfo,
                debug_assertions: profile.debug_assertions,
                overflow_checks: profile.overflow_checks,
                test: unit_mode.is_any_test(),
            };

            machine_message::emit(&machine_message::Artifact {
                package_id: &package_id,
                target: &target,
                profile: art_profile,
                features,
                filenames: destinations,
                fresh,
            });
        }
        Ok(())
    }))
}

fn hardlink_or_copy(src: &Path, dst: &Path) -> CargoResult<()> {
    debug!("linking {} to {}", src.display(), dst.display());
    if is_same_file(src, dst).unwrap_or(false) {
        return Ok(());
    }
    if dst.exists() {
        paths::remove_file(&dst)?;
    }

    let link_result = if src.is_dir() {
        #[cfg(target_os = "redox")]
        use std::os::redox::fs::symlink;
        #[cfg(unix)]
        use std::os::unix::fs::symlink;
        #[cfg(windows)]
        use std::os::windows::fs::symlink_dir as symlink;

        let dst_dir = dst.parent().unwrap();
        let src = if src.starts_with(dst_dir) {
            src.strip_prefix(dst_dir).unwrap()
        } else {
            src
        };
        symlink(src, dst)
    } else {
        fs::hard_link(src, dst)
    };
    link_result
        .or_else(|err| {
            debug!("link failed {}. falling back to fs::copy", err);
            fs::copy(src, dst).map(|_| ())
        })
        .chain_err(|| {
            format!(
                "failed to link or copy `{}` to `{}`",
                src.display(),
                dst.display()
            )
        })?;
    Ok(())
}

fn load_build_deps(cx: &Context, unit: &Unit) -> Option<Arc<BuildScripts>> {
    cx.build_scripts.get(unit).cloned()
}

// For all plugin dependencies, add their -L paths (now calculated and
// present in `state`) to the dynamic library load path for the command to
// execute.
fn add_plugin_deps(
    rustc: &mut ProcessBuilder,
    build_state: &BuildMap,
    build_scripts: &BuildScripts,
    root_output: &PathBuf,
) -> CargoResult<()> {
    let var = util::dylib_path_envvar();
    let search_path = rustc.get_env(var).unwrap_or_default();
    let mut search_path = env::split_paths(&search_path).collect::<Vec<_>>();
    for id in build_scripts.plugins.iter() {
        let key = (id.clone(), Kind::Host);
        let output = build_state
            .get(&key)
            .ok_or_else(|| internal(format!("couldn't find libs for plugin dep {}", id)))?;
        search_path.append(&mut filter_dynamic_search_path(
            output.library_paths.iter(),
            root_output,
        ));
    }
    let search_path = join_paths(&search_path, var)?;
    rustc.env(var, &search_path);
    Ok(())
}

// Determine paths to add to the dynamic search path from -L entries
//
// Strip off prefixes like "native=" or "framework=" and filter out directories
// *not* inside our output directory since they are likely spurious and can cause
// clashes with system shared libraries (issue #3366).
fn filter_dynamic_search_path<'a, I>(paths: I, root_output: &PathBuf) -> Vec<PathBuf>
where
    I: Iterator<Item = &'a PathBuf>,
{
    let mut search_path = vec![];
    for dir in paths {
        let dir = match dir.to_str() {
            Some(s) => {
                let mut parts = s.splitn(2, '=');
                match (parts.next(), parts.next()) {
                    (Some("native"), Some(path))
                    | (Some("crate"), Some(path))
                    | (Some("dependency"), Some(path))
                    | (Some("framework"), Some(path))
                    | (Some("all"), Some(path)) => path.into(),
                    _ => dir.clone(),
                }
            }
            None => dir.clone(),
        };
        if dir.starts_with(&root_output) {
            search_path.push(dir);
        } else {
            debug!(
                "Not including path {} in runtime library search path because it is \
                 outside target root {}",
                dir.display(),
                root_output.display()
            );
        }
    }
    search_path
}

fn prepare_rustc<'a, 'cfg>(
    cx: &mut Context<'a, 'cfg>,
    crate_types: &[&str],
    unit: &Unit<'a>,
) -> CargoResult<ProcessBuilder> {
    let mut base = cx.compilation.rustc_process(unit.pkg)?;
    base.inherit_jobserver(&cx.jobserver);
    build_base_args(cx, &mut base, unit, crate_types)?;
    build_deps_args(&mut base, cx, unit)?;
    Ok(base)
}

fn rustdoc<'a, 'cfg>(cx: &mut Context<'a, 'cfg>, unit: &Unit<'a>) -> CargoResult<Work> {
    let bcx = cx.bcx;
    let mut rustdoc = cx.compilation.rustdoc_process(unit.pkg)?;
    rustdoc.inherit_jobserver(&cx.jobserver);
    rustdoc.arg("--crate-name").arg(&unit.target.crate_name());
    add_path_args(&cx.bcx, unit, &mut rustdoc);

    if unit.kind != Kind::Host {
        if let Some(ref target) = bcx.build_config.requested_target {
            rustdoc.arg("--target").arg(target);
        }
    }

    let doc_dir = cx.files().out_dir(unit);

    // Create the documentation directory ahead of time as rustdoc currently has
    // a bug where concurrent invocations will race to create this directory if
    // it doesn't already exist.
    fs::create_dir_all(&doc_dir)?;

    rustdoc.arg("-o").arg(doc_dir);

    for feat in bcx.resolve.features_sorted(unit.pkg.package_id()) {
        rustdoc.arg("--cfg").arg(&format!("feature=\"{}\"", feat));
    }

    if let Some(ref args) = bcx.extra_args_for(unit) {
        rustdoc.args(args);
    }

    build_deps_args(&mut rustdoc, cx, unit)?;

    rustdoc.args(&bcx.rustdocflags_args(unit)?);

    let name = unit.pkg.name().to_string();
    let build_state = cx.build_state.clone();
    let key = (unit.pkg.package_id().clone(), unit.kind);

    Ok(Work::new(move |state| {
        if let Some(output) = build_state.outputs.lock().unwrap().get(&key) {
            for cfg in output.cfgs.iter() {
                rustdoc.arg("--cfg").arg(cfg);
            }
            for &(ref name, ref value) in output.env.iter() {
                rustdoc.env(name, value);
            }
        }
        state.running(&rustdoc);
        rustdoc
            .exec()
            .chain_err(|| format!("Could not document `{}`.", name))?;
        Ok(())
    }))
}

// The path that we pass to rustc is actually fairly important because it will
// show up in error messages (important for readability), debug information
// (important for caching), etc. As a result we need to be pretty careful how we
// actually invoke rustc.
//
// In general users don't expect `cargo build` to cause rebuilds if you change
// directories. That could be if you just change directories in the project or
// if you literally move the whole project wholesale to a new directory. As a
// result we mostly don't factor in `cwd` to this calculation. Instead we try to
// track the workspace as much as possible and we update the current directory
// of rustc/rustdoc where appropriate.
//
// The first returned value here is the argument to pass to rustc, and the
// second is the cwd that rustc should operate in.
fn path_args(bcx: &BuildContext, unit: &Unit) -> (PathBuf, PathBuf) {
    let ws_root = bcx.ws.root();
    let src = unit.target.src_path();
    assert!(src.is_absolute());
    match src.strip_prefix(ws_root) {
        Ok(path) => (path.to_path_buf(), ws_root.to_path_buf()),
        Err(_) => (src.to_path_buf(), unit.pkg.root().to_path_buf()),
    }
}

fn add_path_args(bcx: &BuildContext, unit: &Unit, cmd: &mut ProcessBuilder) {
    let (arg, cwd) = path_args(bcx, unit);
    cmd.arg(arg);
    cmd.cwd(cwd);
}

fn build_base_args<'a, 'cfg>(
    cx: &mut Context<'a, 'cfg>,
    cmd: &mut ProcessBuilder,
    unit: &Unit<'a>,
    crate_types: &[&str],
) -> CargoResult<()> {
    assert!(!unit.mode.is_run_custom_build());

    let bcx = cx.bcx;
    let Profile {
        ref opt_level,
        ref lto,
        codegen_units,
        debuginfo,
        debug_assertions,
        overflow_checks,
        rpath,
        ref panic,
        ..
    } = unit.profile;
    let test = unit.mode.is_any_test();

    cmd.arg("--crate-name").arg(&unit.target.crate_name());

    add_path_args(&cx.bcx, unit, cmd);

    match bcx.config.shell().color_choice() {
        ColorChoice::Always => {
            cmd.arg("--color").arg("always");
        }
        ColorChoice::Never => {
            cmd.arg("--color").arg("never");
        }
        ColorChoice::CargoAuto => {}
    }

    if bcx.build_config.json_messages() {
        cmd.arg("--error-format").arg("json");
    }

    if !test {
        for crate_type in crate_types.iter() {
            cmd.arg("--crate-type").arg(crate_type);
        }
    }

    if unit.mode.is_check() {
        cmd.arg("--emit=dep-info,metadata");
    } else {
        cmd.arg("--emit=dep-info,link");
    }

    let prefer_dynamic = (unit.target.for_host() && !unit.target.is_custom_build())
        || (crate_types.contains(&"dylib") && bcx.ws.members().any(|p| p != unit.pkg));
    if prefer_dynamic {
        cmd.arg("-C").arg("prefer-dynamic");
    }

    if opt_level.as_str() != "0" {
        cmd.arg("-C").arg(&format!("opt-level={}", opt_level));
    }

    // If a panic mode was configured *and* we're not ever going to be used in a
    // plugin, then we can compile with that panic mode.
    //
    // If we're used in a plugin then we'll eventually be linked to libsyntax
    // most likely which isn't compiled with a custom panic mode, so we'll just
    // get an error if we actually compile with that. This fixes `panic=abort`
    // crates which have plugin dependencies, but unfortunately means that
    // dependencies shared between the main application and plugins must be
    // compiled without `panic=abort`. This isn't so bad, though, as the main
    // application will still be compiled with `panic=abort`.
    if let Some(panic) = panic.as_ref() {
        if !cx.used_in_plugin.contains(unit) {
            cmd.arg("-C").arg(format!("panic={}", panic));
        }
    }

    // Disable LTO for host builds as prefer_dynamic and it are mutually
    // exclusive.
    if unit.target.can_lto() && !unit.target.for_host() {
        match *lto {
            Lto::Bool(false) => {}
            Lto::Bool(true) => {
                cmd.args(&["-C", "lto"]);
            }
            Lto::Named(ref s) => {
                cmd.arg("-C").arg(format!("lto={}", s));
            }
        }
    }

    if let Some(n) = codegen_units {
        // There are some restrictions with LTO and codegen-units, so we
        // only add codegen units when LTO is not used.
        cmd.arg("-C").arg(&format!("codegen-units={}", n));
    }

    if let Some(debuginfo) = debuginfo {
        cmd.arg("-C").arg(format!("debuginfo={}", debuginfo));
    }

    if let Some(ref args) = bcx.extra_args_for(unit) {
        cmd.args(args);
    }

    // -C overflow-checks is implied by the setting of -C debug-assertions,
    // so we only need to provide -C overflow-checks if it differs from
    // the value of -C debug-assertions we would provide.
    if opt_level.as_str() != "0" {
        if debug_assertions {
            cmd.args(&["-C", "debug-assertions=on"]);
            if !overflow_checks {
                cmd.args(&["-C", "overflow-checks=off"]);
            }
        } else if overflow_checks {
            cmd.args(&["-C", "overflow-checks=on"]);
        }
    } else if !debug_assertions {
        cmd.args(&["-C", "debug-assertions=off"]);
        if overflow_checks {
            cmd.args(&["-C", "overflow-checks=on"]);
        }
    } else if !overflow_checks {
        cmd.args(&["-C", "overflow-checks=off"]);
    }

    if test && unit.target.harness() {
        cmd.arg("--test");
    } else if test {
        cmd.arg("--cfg").arg("test");
    }

    // We ideally want deterministic invocations of rustc to ensure that
    // rustc-caching strategies like sccache are able to cache more, so sort the
    // feature list here.
    for feat in bcx.resolve.features_sorted(unit.pkg.package_id()) {
        cmd.arg("--cfg").arg(&format!("feature=\"{}\"", feat));
    }

    match cx.files().metadata(unit) {
        Some(m) => {
            cmd.arg("-C").arg(&format!("metadata={}", m));
            cmd.arg("-C").arg(&format!("extra-filename=-{}", m));
        }
        None => {
            cmd.arg("-C")
                .arg(&format!("metadata={}", cx.files().target_short_hash(unit)));
        }
    }

    if rpath {
        cmd.arg("-C").arg("rpath");
    }

    cmd.arg("--out-dir").arg(&cx.files().out_dir(unit));

    fn opt(cmd: &mut ProcessBuilder, key: &str, prefix: &str, val: Option<&OsStr>) {
        if let Some(val) = val {
            let mut joined = OsString::from(prefix);
            joined.push(val);
            cmd.arg(key).arg(joined);
        }
    }

    if unit.kind == Kind::Target {
        opt(
            cmd,
            "--target",
            "",
            bcx.build_config
                .requested_target
                .as_ref()
                .map(|s| s.as_ref()),
        );
    }

    opt(cmd, "-C", "ar=", bcx.ar(unit.kind).map(|s| s.as_ref()));
    opt(
        cmd,
        "-C",
        "linker=",
        bcx.linker(unit.kind).map(|s| s.as_ref()),
    );
    cmd.args(&cx.incremental_args(unit)?);

    Ok(())
}

fn build_deps_args<'a, 'cfg>(
    cmd: &mut ProcessBuilder,
    cx: &mut Context<'a, 'cfg>,
    unit: &Unit<'a>,
) -> CargoResult<()> {
    let bcx = cx.bcx;
    cmd.arg("-L").arg(&{
        let mut deps = OsString::from("dependency=");
        deps.push(cx.files().deps_dir(unit));
        deps
    });

    // Be sure that the host path is also listed. This'll ensure that proc-macro
    // dependencies are correctly found (for reexported macros).
    if let Kind::Target = unit.kind {
        cmd.arg("-L").arg(&{
            let mut deps = OsString::from("dependency=");
            deps.push(cx.files().host_deps());
            deps
        });
    }

    let dep_targets = cx.dep_targets(unit);

    // If there is not one linkable target but should, rustc fails later
    // on if there is an `extern crate` for it. This may turn into a hard
    // error in the future, see PR #4797
    if !dep_targets
        .iter()
        .any(|u| !u.mode.is_doc() && u.target.linkable())
    {
        if let Some(u) = dep_targets
            .iter()
            .find(|u| !u.mode.is_doc() && u.target.is_lib())
        {
            bcx.config.shell().warn(format!(
                "The package `{}` \
                 provides no linkable target. The compiler might raise an error while compiling \
                 `{}`. Consider adding 'dylib' or 'rlib' to key `crate-type` in `{}`'s \
                 Cargo.toml. This warning might turn into a hard error in the future.",
                u.target.crate_name(),
                unit.target.crate_name(),
                u.target.crate_name()
            ))?;
        }
    }

    for dep in dep_targets {
        if dep.mode.is_run_custom_build() {
            cmd.env("OUT_DIR", &cx.files().build_script_out_dir(&dep));
        }
        if dep.target.linkable() && !dep.mode.is_doc() {
            link_to(cmd, cx, unit, &dep)?;
        }
    }

    return Ok(());

    fn link_to<'a, 'cfg>(
        cmd: &mut ProcessBuilder,
        cx: &mut Context<'a, 'cfg>,
        current: &Unit<'a>,
        dep: &Unit<'a>,
    ) -> CargoResult<()> {
        let bcx = cx.bcx;
        for output in cx.outputs(dep)?.iter() {
            if output.flavor != FileFlavor::Linkable {
                continue;
            }
            let mut v = OsString::new();
            let name = bcx.extern_crate_name(current, dep)?;
            v.push(name);
            v.push("=");
            v.push(cx.files().out_dir(dep));
            v.push(&path::MAIN_SEPARATOR.to_string());
            v.push(&output.path.file_name().unwrap());
            cmd.arg("--extern").arg(&v);
        }
        Ok(())
    }
}

fn envify(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_uppercase())
        .map(|c| if c == '-' { '_' } else { c })
        .collect()
}

impl Kind {
    fn for_target(&self, target: &Target) -> Kind {
        // Once we start compiling for the `Host` kind we continue doing so, but
        // if we are a `Target` kind and then we start compiling for a target
        // that needs to be on the host we lift ourselves up to `Host`
        match *self {
            Kind::Host => Kind::Host,
            Kind::Target if target.for_host() => Kind::Host,
            Kind::Target => Kind::Target,
        }
    }
}
