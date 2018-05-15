use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;

use semver::Version;
use lazycell::LazyCell;

use core::{Feature, Package, PackageId, Target, TargetKind};
use util::{self, join_paths, process, CargoResult, Config, ProcessBuilder};
use super::BuildContext;

/// A structure returning the result of a compilation.
pub struct Compilation<'cfg> {
    /// A mapping from a package to the list of libraries that need to be
    /// linked when working with that package.
    pub libraries: HashMap<PackageId, HashSet<(Target, PathBuf)>>,

    /// An array of all tests created during this compilation.
    pub tests: Vec<(Package, TargetKind, String, PathBuf)>,

    /// An array of all binaries created.
    pub binaries: Vec<PathBuf>,

    /// All directories for the output of native build commands.
    ///
    /// This is currently used to drive some entries which are added to the
    /// LD_LIBRARY_PATH as appropriate.
    ///
    /// The order should be deterministic.
    // TODO: deprecated, remove
    pub native_dirs: BTreeSet<PathBuf>,

    /// Root output directory (for the local package's artifacts)
    pub root_output: PathBuf,

    /// Output directory for rust dependencies.
    /// May be for the host or for a specific target.
    pub deps_output: PathBuf,

    /// Output directory for the rust host dependencies.
    pub host_deps_output: PathBuf,

    /// The path to rustc's own libstd
    pub host_dylib_path: Option<PathBuf>,

    /// The path to libstd for the target
    pub target_dylib_path: Option<PathBuf>,

    /// Extra environment variables that were passed to compilations and should
    /// be passed to future invocations of programs.
    pub extra_env: HashMap<PackageId, Vec<(String, String)>>,

    pub to_doc_test: Vec<Package>,

    /// Features per package enabled during this compilation.
    pub cfgs: HashMap<PackageId, HashSet<String>>,

    /// Flags to pass to rustdoc when invoked from cargo test, per package.
    pub rustdocflags: HashMap<PackageId, Vec<String>>,

    pub host: String,
    pub target: String,

    config: &'cfg Config,
    rustc_process: ProcessBuilder,

    target_runner: LazyCell<Option<(PathBuf, Vec<String>)>>,
}

impl<'cfg> Compilation<'cfg> {
    pub fn new<'a>(bcx: &BuildContext<'a, 'cfg>) -> Compilation<'cfg> {
        Compilation {
            libraries: HashMap::new(),
            native_dirs: BTreeSet::new(), // TODO: deprecated, remove
            root_output: PathBuf::from("/"),
            deps_output: PathBuf::from("/"),
            host_deps_output: PathBuf::from("/"),
            host_dylib_path: bcx.host_info.sysroot_libdir.clone(),
            target_dylib_path: bcx.target_info.sysroot_libdir.clone(),
            tests: Vec::new(),
            binaries: Vec::new(),
            extra_env: HashMap::new(),
            to_doc_test: Vec::new(),
            cfgs: HashMap::new(),
            rustdocflags: HashMap::new(),
            config: bcx.config,
            rustc_process: bcx.rustc.process(),
            host: bcx.host_triple().to_string(),
            target: bcx.target_triple().to_string(),
            target_runner: LazyCell::new(),
        }
    }

    /// See `process`.
    pub fn rustc_process(&self, pkg: &Package) -> CargoResult<ProcessBuilder> {
        let mut p = self.fill_env(self.rustc_process.clone(), pkg, true)?;
        let manifest = pkg.manifest();
        if manifest.features().is_enabled(Feature::edition()) {
            p.arg(format!("--edition={}", manifest.edition()));
        }
        Ok(p)
    }

    /// See `process`.
    pub fn rustdoc_process(&self, pkg: &Package) -> CargoResult<ProcessBuilder> {
        let mut p = self.fill_env(process(&*self.config.rustdoc()?), pkg, false)?;
        let manifest = pkg.manifest();
        if manifest.features().is_enabled(Feature::edition()) {
            p.arg("-Zunstable-options");
            p.arg(format!("--edition={}", &manifest.edition()));
        }
        Ok(p)
    }

    /// See `process`.
    pub fn host_process<T: AsRef<OsStr>>(
        &self,
        cmd: T,
        pkg: &Package,
    ) -> CargoResult<ProcessBuilder> {
        self.fill_env(process(cmd), pkg, true)
    }

    fn target_runner(&self) -> CargoResult<&Option<(PathBuf, Vec<String>)>> {
        self.target_runner.try_borrow_with(|| {
            let key = format!("target.{}.runner", self.target);
            Ok(self.config.get_path_and_args(&key)?.map(|v| v.val))
        })
    }

    /// See `process`.
    pub fn target_process<T: AsRef<OsStr>>(
        &self,
        cmd: T,
        pkg: &Package,
    ) -> CargoResult<ProcessBuilder> {
        let builder = if let Some((ref runner, ref args)) = *self.target_runner()? {
            let mut builder = process(runner);
            builder.args(args);
            builder.arg(cmd);
            builder
        } else {
            process(cmd)
        };
        self.fill_env(builder, pkg, false)
    }

    /// Prepares a new process with an appropriate environment to run against
    /// the artifacts produced by the build process.
    ///
    /// The package argument is also used to configure environment variables as
    /// well as the working directory of the child process.
    fn fill_env(
        &self,
        mut cmd: ProcessBuilder,
        pkg: &Package,
        is_host: bool,
    ) -> CargoResult<ProcessBuilder> {
        let mut search_path = if is_host {
            let mut search_path = vec![self.host_deps_output.clone()];
            search_path.extend(self.host_dylib_path.clone());
            search_path
        } else {
            let mut search_path =
                super::filter_dynamic_search_path(self.native_dirs.iter(), &self.root_output);
            search_path.push(self.root_output.clone());
            search_path.push(self.deps_output.clone());
            search_path.extend(self.target_dylib_path.clone());
            search_path
        };

        search_path.extend(util::dylib_path().into_iter());
        let search_path = join_paths(&search_path, util::dylib_path_envvar())?;

        cmd.env(util::dylib_path_envvar(), &search_path);
        if let Some(env) = self.extra_env.get(pkg.package_id()) {
            for &(ref k, ref v) in env {
                cmd.env(k, v);
            }
        }

        let metadata = pkg.manifest().metadata();

        let cargo_exe = self.config.cargo_exe()?;
        cmd.env(::CARGO_ENV, cargo_exe);

        // When adding new environment variables depending on
        // crate properties which might require rebuild upon change
        // consider adding the corresponding properties to the hash
        // in BuildContext::target_metadata()
        cmd.env("CARGO_MANIFEST_DIR", pkg.root())
            .env("CARGO_PKG_VERSION_MAJOR", &pkg.version().major.to_string())
            .env("CARGO_PKG_VERSION_MINOR", &pkg.version().minor.to_string())
            .env("CARGO_PKG_VERSION_PATCH", &pkg.version().patch.to_string())
            .env(
                "CARGO_PKG_VERSION_PRE",
                &pre_version_component(pkg.version()),
            )
            .env("CARGO_PKG_VERSION", &pkg.version().to_string())
            .env("CARGO_PKG_NAME", &*pkg.name())
            .env(
                "CARGO_PKG_DESCRIPTION",
                metadata.description.as_ref().unwrap_or(&String::new()),
            )
            .env(
                "CARGO_PKG_HOMEPAGE",
                metadata.homepage.as_ref().unwrap_or(&String::new()),
            )
            .env("CARGO_PKG_AUTHORS", &pkg.authors().join(":"))
            .cwd(pkg.root());
        Ok(cmd)
    }
}

fn pre_version_component(v: &Version) -> String {
    if v.pre.is_empty() {
        return String::new();
    }

    let mut ret = String::new();

    for (i, x) in v.pre.iter().enumerate() {
        if i != 0 {
            ret.push('.')
        };
        ret.push_str(&x.to_string());
    }

    ret
}
