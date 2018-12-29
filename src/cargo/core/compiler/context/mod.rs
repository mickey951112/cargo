#![allow(deprecated)]
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use jobserver::Client;

use crate::core::compiler::compilation;
use crate::core::profiles::Profile;
use crate::core::{Package, PackageId, Resolve, Target};
use crate::util::errors::{CargoResult, CargoResultExt};
use crate::util::{internal, profile, short_hash, Config};

use super::build_plan::BuildPlan;
use super::custom_build::{self, BuildDeps, BuildScripts, BuildState};
use super::fingerprint::Fingerprint;
use super::job_queue::JobQueue;
use super::layout::Layout;
use super::{BuildContext, Compilation, CompileMode, Executor, FileFlavor, Kind};

mod unit_dependencies;
use self::unit_dependencies::build_unit_dependencies;

mod compilation_files;
use self::compilation_files::CompilationFiles;
pub use self::compilation_files::{Metadata, OutputFile};

/// All information needed to define a Unit.
///
/// A unit is an object that has enough information so that cargo knows how to build it.
/// For example, if your package has dependencies, then every dependency will be built as a library
/// unit. If your package is a library, then it will be built as a library unit as well, or if it
/// is a binary with `main.rs`, then a binary will be output. There are also separate unit types
/// for `test`ing and `check`ing, amongst others.
///
/// The unit also holds information about all possible metadata about the package in `pkg`.
///
/// A unit needs to know extra information in addition to the type and root source file. For
/// example, it needs to know the target architecture (OS, chip arch etc.) and it needs to know
/// whether you want a debug or release build. There is enough information in this struct to figure
/// all that out.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, PartialOrd, Ord)]
pub struct Unit<'a> {
    /// Information about available targets, which files to include/exclude, etc. Basically stuff in
    /// `Cargo.toml`.
    pub pkg: &'a Package,
    /// Information about the specific target to build, out of the possible targets in `pkg`. Not
    /// to be confused with *target-triple* (or *target architecture* ...), the target arch for a
    /// build.
    pub target: &'a Target,
    /// The profile contains information about *how* the build should be run, including debug
    /// level, etc.
    pub profile: Profile,
    /// Whether this compilation unit is for the host or target architecture.
    ///
    /// For example, when
    /// cross compiling and using a custom build script, the build script needs to be compiled for
    /// the host architecture so the host rustc can use it (when compiling to the target
    /// architecture).
    pub kind: Kind,
    /// The "mode" this unit is being compiled for.  See `CompileMode` for
    /// more details.
    pub mode: CompileMode,
}

impl<'a> Unit<'a> {
    pub fn buildkey(&self) -> String {
        format!("{}-{}", self.pkg.name(), short_hash(self))
    }
}

pub struct Context<'a, 'cfg: 'a> {
    pub bcx: &'a BuildContext<'a, 'cfg>,
    pub compilation: Compilation<'cfg>,
    pub build_state: Arc<BuildState>,
    pub build_script_overridden: HashSet<(PackageId, Kind)>,
    pub build_explicit_deps: HashMap<Unit<'a>, BuildDeps>,
    pub fingerprints: HashMap<Unit<'a>, Arc<Fingerprint>>,
    pub compiled: HashSet<Unit<'a>>,
    pub build_scripts: HashMap<Unit<'a>, Arc<BuildScripts>>,
    pub links: Links,
    pub jobserver: Client,
    primary_packages: HashSet<PackageId>,
    unit_dependencies: HashMap<Unit<'a>, Vec<Unit<'a>>>,
    files: Option<CompilationFiles<'a, 'cfg>>,
    package_cache: HashMap<PackageId, &'a Package>,
}

impl<'a, 'cfg> Context<'a, 'cfg> {
    pub fn new(config: &'cfg Config, bcx: &'a BuildContext<'a, 'cfg>) -> CargoResult<Self> {
        // Load up the jobserver that we'll use to manage our parallelism. This
        // is the same as the GNU make implementation of a jobserver, and
        // intentionally so! It's hoped that we can interact with GNU make and
        // all share the same jobserver.
        //
        // Note that if we don't have a jobserver in our environment then we
        // create our own, and we create it with `n-1` tokens because one token
        // is ourself, a running process.
        let jobserver = match config.jobserver_from_env() {
            Some(c) => c.clone(),
            None => Client::new(bcx.build_config.jobs as usize - 1)
                .chain_err(|| "failed to create jobserver")?,
        };

        Ok(Self {
            bcx,
            compilation: Compilation::new(bcx)?,
            build_state: Arc::new(BuildState::new(&bcx.host_config, &bcx.target_config)),
            fingerprints: HashMap::new(),
            compiled: HashSet::new(),
            build_scripts: HashMap::new(),
            build_explicit_deps: HashMap::new(),
            links: Links::new(),
            jobserver,
            build_script_overridden: HashSet::new(),

            primary_packages: HashSet::new(),
            unit_dependencies: HashMap::new(),
            files: None,
            package_cache: HashMap::new(),
        })
    }

    // Returns a mapping of the root package plus its immediate dependencies to
    // where the compiled libraries are all located.
    pub fn compile(
        mut self,
        units: &[Unit<'a>],
        export_dir: Option<PathBuf>,
        exec: &Arc<dyn Executor>,
    ) -> CargoResult<Compilation<'cfg>> {
        let mut queue = JobQueue::new(self.bcx);
        let mut plan = BuildPlan::new();
        let build_plan = self.bcx.build_config.build_plan;
        self.prepare_units(export_dir, units)?;
        self.prepare()?;
        custom_build::build_map(&mut self, units)?;
        self.check_collistions()?;

        for unit in units.iter() {
            // Build up a list of pending jobs, each of which represent
            // compiling a particular package. No actual work is executed as
            // part of this, that's all done next as part of the `execute`
            // function which will run everything in order with proper
            // parallelism.
            let force_rebuild = self.bcx.build_config.force_rebuild;
            super::compile(&mut self, &mut queue, &mut plan, unit, exec, force_rebuild)?;
        }

        // Now that we've figured out everything that we're going to do, do it!
        queue.execute(&mut self, &mut plan)?;

        if build_plan {
            plan.set_inputs(self.build_plan_inputs()?);
            plan.output_plan();
        }

        for unit in units.iter() {
            for output in self.outputs(unit)?.iter() {
                if output.flavor == FileFlavor::DebugInfo {
                    continue;
                }

                let bindst = output.bin_dst();

                if unit.mode == CompileMode::Test {
                    self.compilation.tests.push((
                        unit.pkg.clone(),
                        unit.target.kind().clone(),
                        unit.target.name().to_string(),
                        output.path.clone(),
                    ));
                } else if unit.target.is_bin() || unit.target.is_bin_example() {
                    self.compilation.binaries.push(bindst.clone());
                }
            }

            for dep in self.dep_targets(unit).iter() {
                if !unit.target.is_lib() {
                    continue;
                }

                if dep.mode.is_run_custom_build() {
                    let out_dir = self.files().build_script_out_dir(dep).display().to_string();
                    self.compilation
                        .extra_env
                        .entry(dep.pkg.package_id())
                        .or_insert_with(Vec::new)
                        .push(("OUT_DIR".to_string(), out_dir));
                }
            }

            if unit.mode == CompileMode::Doctest {
                // Note that we can *only* doctest rlib outputs here.  A
                // staticlib output cannot be linked by the compiler (it just
                // doesn't do that). A dylib output, however, can be linked by
                // the compiler, but will always fail. Currently all dylibs are
                // built as "static dylibs" where the standard library is
                // statically linked into the dylib. The doc tests fail,
                // however, for now as they try to link the standard library
                // dynamically as well, causing problems. As a result we only
                // pass `--extern` for rlib deps and skip out on all other
                // artifacts.
                let mut doctest_deps = Vec::new();
                for dep in self.dep_targets(unit) {
                    if dep.target.is_lib() && dep.mode == CompileMode::Build {
                        let outputs = self.outputs(&dep)?;
                        let outputs = outputs.iter().filter(|output| {
                            output.path.extension() == Some(OsStr::new("rlib"))
                                || dep.target.for_host()
                        });
                        for output in outputs {
                            doctest_deps.push((
                                self.bcx.extern_crate_name(unit, &dep)?,
                                output.path.clone(),
                            ));
                        }
                    }
                }
                // Help with tests to get a stable order with renamed deps.
                doctest_deps.sort();
                self.compilation.to_doc_test.push(compilation::Doctest {
                    package: unit.pkg.clone(),
                    target: unit.target.clone(),
                    deps: doctest_deps,
                });
            }

            let feats = self.bcx.resolve.features(unit.pkg.package_id());
            if !feats.is_empty() {
                self.compilation
                    .cfgs
                    .entry(unit.pkg.package_id())
                    .or_insert_with(|| {
                        feats
                            .iter()
                            .map(|feat| format!("feature=\"{}\"", feat))
                            .collect()
                    });
            }
            let rustdocflags = self.bcx.rustdocflags_args(unit)?;
            if !rustdocflags.is_empty() {
                self.compilation
                    .rustdocflags
                    .entry(unit.pkg.package_id())
                    .or_insert(rustdocflags);
            }

            super::output_depinfo(&mut self, unit)?;
        }

        for (&(ref pkg, _), output) in self.build_state.outputs.lock().unwrap().iter() {
            self.compilation
                .cfgs
                .entry(pkg.clone())
                .or_insert_with(HashSet::new)
                .extend(output.cfgs.iter().cloned());

            self.compilation
                .extra_env
                .entry(pkg.clone())
                .or_insert_with(Vec::new)
                .extend(output.env.iter().cloned());

            for dir in output.library_paths.iter() {
                self.compilation.native_dirs.insert(dir.clone());
            }
        }
        Ok(self.compilation)
    }

    /// Returns the executable for the specified unit (if any).
    pub fn get_executable(&mut self, unit: &Unit<'a>) -> CargoResult<Option<PathBuf>> {
        for output in self.outputs(unit)?.iter() {
            if output.flavor == FileFlavor::DebugInfo {
                continue;
            }

            let is_binary = unit.target.is_bin() || unit.target.is_bin_example();
            let is_test = unit.mode.is_any_test() && !unit.mode.is_check();

            if is_binary || is_test {
                return Ok(Option::Some(output.bin_dst().clone()));
            }
        }

        Ok(None)
    }

    pub fn prepare_units(
        &mut self,
        export_dir: Option<PathBuf>,
        units: &[Unit<'a>],
    ) -> CargoResult<()> {
        let dest = if self.bcx.build_config.release {
            "release"
        } else {
            "debug"
        };
        let host_layout = Layout::new(self.bcx.ws, None, dest)?;
        let target_layout = match self.bcx.build_config.requested_target.as_ref() {
            Some(target) => Some(Layout::new(self.bcx.ws, Some(target), dest)?),
            None => None,
        };
        self.primary_packages
            .extend(units.iter().map(|u| u.pkg.package_id()));

        build_unit_dependencies(
            units,
            self.bcx,
            &mut self.unit_dependencies,
            &mut self.package_cache,
        )?;
        let files = CompilationFiles::new(
            units,
            host_layout,
            target_layout,
            export_dir,
            self.bcx.ws,
            self,
        );
        self.files = Some(files);
        Ok(())
    }

    /// Prepare this context, ensuring that all filesystem directories are in
    /// place.
    pub fn prepare(&mut self) -> CargoResult<()> {
        let _p = profile::start("preparing layout");

        self.files_mut()
            .host
            .prepare()
            .chain_err(|| internal("couldn't prepare build directories"))?;
        if let Some(ref mut target) = self.files.as_mut().unwrap().target {
            target
                .prepare()
                .chain_err(|| internal("couldn't prepare build directories"))?;
        }

        self.compilation.host_deps_output = self.files_mut().host.deps().to_path_buf();

        let files = self.files.as_ref().unwrap();
        let layout = files.target.as_ref().unwrap_or(&files.host);
        self.compilation.root_output = layout.dest().to_path_buf();
        self.compilation.deps_output = layout.deps().to_path_buf();
        Ok(())
    }

    pub fn files(&self) -> &CompilationFiles<'a, 'cfg> {
        self.files.as_ref().unwrap()
    }

    fn files_mut(&mut self) -> &mut CompilationFiles<'a, 'cfg> {
        self.files.as_mut().unwrap()
    }

    /// Return the filenames that the given unit will generate.
    pub fn outputs(&self, unit: &Unit<'a>) -> CargoResult<Arc<Vec<OutputFile>>> {
        self.files.as_ref().unwrap().outputs(unit, self.bcx)
    }

    /// For a package, return all targets which are registered as dependencies
    /// for that package.
    // TODO: this ideally should be `-> &[Unit<'a>]`
    pub fn dep_targets(&self, unit: &Unit<'a>) -> Vec<Unit<'a>> {
        // If this build script's execution has been overridden then we don't
        // actually depend on anything, we've reached the end of the dependency
        // chain as we've got all the info we're gonna get.
        //
        // Note there's a subtlety about this piece of code! The
        // `build_script_overridden` map here is populated in
        // `custom_build::build_map` which you need to call before inspecting
        // dependencies. However, that code itself calls this method and
        // gets a full pre-filtered set of dependencies. This is not super
        // obvious, and clear, but it does work at the moment.
        if unit.target.is_custom_build() {
            let key = (unit.pkg.package_id(), unit.kind);
            if self.build_script_overridden.contains(&key) {
                return Vec::new();
            }
        }
        let mut deps = self.unit_dependencies[unit].clone();
        deps.sort();
        deps
    }

    pub fn incremental_args(&self, unit: &Unit<'_>) -> CargoResult<Vec<String>> {
        // There's a number of ways to configure incremental compilation right
        // now. In order of descending priority (first is highest priority) we
        // have:
        //
        // * `CARGO_INCREMENTAL` - this is blanket used unconditionally to turn
        //   on/off incremental compilation for any cargo subcommand. We'll
        //   respect this if set.
        // * `build.incremental` - in `.cargo/config` this blanket key can
        //   globally for a system configure whether incremental compilation is
        //   enabled. Note that setting this to `true` will not actually affect
        //   all builds though. For example a `true` value doesn't enable
        //   release incremental builds, only dev incremental builds. This can
        //   be useful to globally disable incremental compilation like
        //   `CARGO_INCREMENTAL`.
        // * `profile.dev.incremental` - in `Cargo.toml` specific profiles can
        //   be configured to enable/disable incremental compilation. This can
        //   be primarily used to disable incremental when buggy for a package.
        // * Finally, each profile has a default for whether it will enable
        //   incremental compilation or not. Primarily development profiles
        //   have it enabled by default while release profiles have it disabled
        //   by default.
        let global_cfg = self
            .bcx
            .config
            .get_bool("build.incremental")?
            .map(|c| c.val);
        let incremental = match (
            self.bcx.incremental_env,
            global_cfg,
            unit.profile.incremental,
        ) {
            (Some(v), _, _) => v,
            (None, Some(false), _) => false,
            (None, _, other) => other,
        };

        if !incremental {
            return Ok(Vec::new());
        }

        // Only enable incremental compilation for sources the user can
        // modify (aka path sources). For things that change infrequently,
        // non-incremental builds yield better performance in the compiler
        // itself (aka crates.io / git dependencies)
        //
        // (see also https://github.com/rust-lang/cargo/issues/3972)
        if !unit.pkg.package_id().source_id().is_path() {
            return Ok(Vec::new());
        }

        let dir = self.files().layout(unit.kind).incremental().display();
        Ok(vec!["-C".to_string(), format!("incremental={}", dir)])
    }

    pub fn is_primary_package(&self, unit: &Unit<'a>) -> bool {
        self.primary_packages.contains(&unit.pkg.package_id())
    }

    /// Gets a package for the given package id.
    pub fn get_package(&self, id: PackageId) -> CargoResult<&'a Package> {
        self.package_cache
            .get(&id)
            .cloned()
            .ok_or_else(|| failure::format_err!("failed to find {}", id))
    }

    /// Return the list of filenames read by cargo to generate the BuildContext
    /// (all Cargo.toml, etc).
    pub fn build_plan_inputs(&self) -> CargoResult<Vec<PathBuf>> {
        let mut inputs = Vec::new();
        // Note that we're using the `package_cache`, which should have been
        // populated by `build_unit_dependencies`, and only those packages are
        // considered as all the inputs.
        //
        // (notably we skip dev-deps here if they aren't present)
        for pkg in self.package_cache.values() {
            inputs.push(pkg.manifest_path().to_path_buf());
        }
        inputs.sort();
        Ok(inputs)
    }

    fn check_collistions(&self) -> CargoResult<()> {
        let mut output_collisions = HashMap::new();
        let describe_collision =
            |unit: &Unit<'_>, other_unit: &Unit<'_>, path: &PathBuf| -> String {
                format!(
                    "The {} target `{}` in package `{}` has the same output \
                     filename as the {} target `{}` in package `{}`.\n\
                     Colliding filename is: {}\n",
                    unit.target.kind().description(),
                    unit.target.name(),
                    unit.pkg.package_id(),
                    other_unit.target.kind().description(),
                    other_unit.target.name(),
                    other_unit.pkg.package_id(),
                    path.display()
                )
            };
        let suggestion = "Consider changing their names to be unique or compiling them separately.\n\
            This may become a hard error in the future, see https://github.com/rust-lang/cargo/issues/6313";
        let report_collision = |unit: &Unit<'_>,
                                other_unit: &Unit<'_>,
                                path: &PathBuf|
         -> CargoResult<()> {
            if unit.target.name() == other_unit.target.name() {
                self.bcx.config.shell().warn(format!(
                    "output filename collision.\n\
                     {}\
                     The targets should have unique names.\n\
                     {}",
                    describe_collision(unit, other_unit, path),
                    suggestion
                ))
            } else {
                self.bcx.config.shell().warn(format!(
                    "output filename collision.\n\
                    {}\
                    The output filenames should be unique.\n\
                    {}\n\
                    If this looks unexpected, it may be a bug in Cargo. Please file a bug report at\n\
                    https://github.com/rust-lang/cargo/issues/ with as much information as you\n\
                    can provide.\n\
                    {} running on `{}` target `{}`\n\
                    First unit: {:?}\n\
                    Second unit: {:?}",
                    describe_collision(unit, other_unit, path),
                    suggestion,
                    crate::version(), self.bcx.host_triple(), self.bcx.target_triple(),
                    unit, other_unit))
            }
        };
        let mut keys = self
            .unit_dependencies
            .keys()
            .filter(|unit| !unit.mode.is_run_custom_build())
            .collect::<Vec<_>>();
        // Sort for consistent error messages.
        keys.sort_unstable();
        for unit in keys {
            for output in self.outputs(unit)?.iter() {
                if let Some(other_unit) = output_collisions.insert(output.path.clone(), unit) {
                    report_collision(unit, &other_unit, &output.path)?;
                }
                if let Some(hardlink) = output.hardlink.as_ref() {
                    if let Some(other_unit) = output_collisions.insert(hardlink.clone(), unit) {
                        report_collision(unit, &other_unit, hardlink)?;
                    }
                }
                if let Some(ref export_path) = output.export_path {
                    if let Some(other_unit) = output_collisions.insert(export_path.clone(), unit) {
                        self.bcx.config.shell().warn(format!(
                            "`--out-dir` filename collision.\n\
                             {}\
                             The exported filenames should be unique.\n\
                             {}",
                            describe_collision(unit, &other_unit, &export_path),
                            suggestion
                        ))?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Links {
    validated: HashSet<PackageId>,
    links: HashMap<String, PackageId>,
}

impl Links {
    pub fn new() -> Links {
        Links {
            validated: HashSet::new(),
            links: HashMap::new(),
        }
    }

    pub fn validate(&mut self, resolve: &Resolve, unit: &Unit<'_>) -> CargoResult<()> {
        if !self.validated.insert(unit.pkg.package_id()) {
            return Ok(());
        }
        let lib = match unit.pkg.manifest().links() {
            Some(lib) => lib,
            None => return Ok(()),
        };
        if let Some(&prev) = self.links.get(lib) {
            let pkg = unit.pkg.package_id();

            let describe_path = |pkgid: PackageId| -> String {
                let dep_path = resolve.path_to_top(&pkgid);
                let mut dep_path_desc = format!("package `{}`", dep_path[0]);
                for dep in dep_path.iter().skip(1) {
                    write!(dep_path_desc, "\n    ... which is depended on by `{}`", dep).unwrap();
                }
                dep_path_desc
            };

            failure::bail!(
                "multiple packages link to native library `{}`, \
                 but a native library can be linked only once\n\
                 \n\
                 {}\nlinks to native library `{}`\n\
                 \n\
                 {}\nalso links to native library `{}`",
                lib,
                describe_path(prev),
                lib,
                describe_path(pkg),
                lib
            )
        }
        if !unit
            .pkg
            .manifest()
            .targets()
            .iter()
            .any(|t| t.is_custom_build())
        {
            failure::bail!(
                "package `{}` specifies that it links to `{}` but does not \
                 have a custom build script",
                unit.pkg.package_id(),
                lib
            )
        }
        self.links.insert(lib.to_string(), unit.pkg.package_id());
        Ok(())
    }
}
