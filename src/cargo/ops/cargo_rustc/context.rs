use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::{HashSet, HashMap};
use std::str;
use std::sync::Arc;
use std::path::PathBuf;

use regex::Regex;

use core::{SourceMap, Package, PackageId, PackageSet, Resolve, Target, Profile};
use core::{TargetKind, LibKind, Profiles, Metadata};
use util::{self, CargoResult, ChainError, internal, Config, profile};
use util::human;

use super::TargetConfig;
use super::custom_build::BuildState;
use super::fingerprint::Fingerprint;
use super::layout::{Layout, LayoutProxy};
use super::{Kind, Compilation, BuildConfig};
use super::{ProcessEngine, ExecEngine};

#[derive(Debug, Clone, Copy)]
pub enum Platform {
    Target,
    Plugin,
    PluginAndTarget,
}

pub struct Context<'a, 'b: 'a> {
    pub config: &'a Config<'b>,
    pub resolve: &'a Resolve,
    pub sources: &'a SourceMap<'a>,
    pub compilation: Compilation,
    pub build_state: Arc<BuildState>,
    pub exec_engine: Arc<Box<ExecEngine>>,
    pub fingerprints: HashMap<(&'a PackageId, &'a Target, &'a Profile, Kind),
                              Fingerprint>,
    pub compiled: HashSet<(&'a PackageId, &'a Target, &'a Profile)>,
    pub build_config: BuildConfig,

    host: Layout,
    target: Option<Layout>,
    target_triple: String,
    host_dylib: Option<(String, String)>,
    host_exe: String,
    package_set: &'a PackageSet,
    target_dylib: Option<(String, String)>,
    target_exe: String,
    requirements: HashMap<(&'a PackageId, &'a str), Platform>,
    profiles: &'a Profiles,
}

impl<'a, 'b: 'a> Context<'a, 'b> {
    pub fn new(resolve: &'a Resolve,
               sources: &'a SourceMap<'a>,
               deps: &'a PackageSet,
               config: &'a Config<'b>,
               host: Layout,
               target_layout: Option<Layout>,
               root_pkg: &Package,
               build_config: BuildConfig,
               profiles: &'a Profiles) -> CargoResult<Context<'a, 'b>> {
        let target = build_config.requested_target.clone();
        let target = target.as_ref().map(|s| &s[..]);
        let (target_dylib, target_exe) = try!(Context::filename_parts(target));
        let (host_dylib, host_exe) = if build_config.requested_target.is_none() {
            (target_dylib.clone(), target_exe.clone())
        } else {
            try!(Context::filename_parts(None))
        };
        let target_triple = target.unwrap_or(config.rustc_host()).to_string();
        let engine = build_config.exec_engine.as_ref().cloned().unwrap_or({
            Arc::new(Box::new(ProcessEngine))
        });
        Ok(Context {
            target_triple: target_triple,
            host: host,
            target: target_layout,
            resolve: resolve,
            sources: sources,
            package_set: deps,
            config: config,
            target_dylib: target_dylib,
            target_exe: target_exe,
            host_dylib: host_dylib,
            host_exe: host_exe,
            requirements: HashMap::new(),
            compilation: Compilation::new(root_pkg),
            build_state: Arc::new(BuildState::new(&build_config, deps)),
            build_config: build_config,
            exec_engine: engine,
            fingerprints: HashMap::new(),
            profiles: profiles,
            compiled: HashSet::new(),
        })
    }

    /// Run `rustc` to discover the dylib prefix/suffix for the target
    /// specified as well as the exe suffix
    fn filename_parts(target: Option<&str>)
                      -> CargoResult<(Option<(String, String)>, String)> {
        let mut process = try!(util::process("rustc"));
        process.arg("-")
               .arg("--crate-name").arg("_")
               .arg("--crate-type").arg("dylib")
               .arg("--crate-type").arg("bin")
               .arg("--print=file-names")
               .env_remove("RUST_LOG");
        if let Some(s) = target {
            process.arg("--target").arg(s);
        };
        let output = try!(process.exec_with_output());

        let error = str::from_utf8(&output.stderr).unwrap();
        let output = str::from_utf8(&output.stdout).unwrap();
        let mut lines = output.lines();
        let nodylib = Regex::new("unsupported crate type.*dylib").unwrap();
        let nobin = Regex::new("unsupported crate type.*bin").unwrap();
        let dylib = if nodylib.is_match(error) {
            None
        } else {
            let dylib_parts: Vec<&str> = lines.next().unwrap().trim()
                                              .split('_').collect();
            assert!(dylib_parts.len() == 2,
                    "rustc --print-file-name output has changed");
            Some((dylib_parts[0].to_string(), dylib_parts[1].to_string()))
        };

        let exe_suffix = if nobin.is_match(error) {
            String::new()
        } else {
            lines.next().unwrap().trim()
                 .split('_').skip(1).next().unwrap().to_string()
        };
        Ok((dylib, exe_suffix.to_string()))
    }

    /// Prepare this context, ensuring that all filesystem directories are in
    /// place.
    pub fn prepare(&mut self, pkg: &'a Package,
                   targets: &[(&'a Target, &'a Profile)])
                   -> CargoResult<()> {
        let _p = profile::start("preparing layout");

        try!(self.host.prepare().chain_error(|| {
            internal(format!("couldn't prepare build directories for `{}`",
                             pkg.name()))
        }));
        match self.target {
            Some(ref mut target) => {
                try!(target.prepare().chain_error(|| {
                    internal(format!("couldn't prepare build directories \
                                      for `{}`", pkg.name()))
                }));
            }
            None => {}
        }

        for &(target, profile) in targets {
            self.build_requirements(pkg, target, profile, Platform::Target);
        }

        let jobs = self.jobs();
        self.compilation.extra_env.insert("NUM_JOBS".to_string(),
                                          jobs.to_string());
        self.compilation.root_output =
                self.layout(pkg, Kind::Target).proxy().dest().to_path_buf();
        self.compilation.deps_output =
                self.layout(pkg, Kind::Target).proxy().deps().to_path_buf();

        return Ok(());
    }

    fn build_requirements(&mut self, pkg: &'a Package, target: &'a Target,
                          profile: &Profile, req: Platform) {
        let req = if target.for_host() {Platform::Plugin} else {req};
        match self.requirements.entry((pkg.package_id(), target.name())) {
            Occupied(mut entry) => match (*entry.get(), req) {
                (Platform::Plugin, Platform::Plugin) |
                (Platform::PluginAndTarget, Platform::Plugin) |
                (Platform::Target, Platform::Target) |
                (Platform::PluginAndTarget, Platform::Target) |
                (Platform::PluginAndTarget, Platform::PluginAndTarget) => return,
                _ => *entry.get_mut() = entry.get().combine(req),
            },
            Vacant(entry) => { entry.insert(req); }
        };

        for &(pkg, dep, profile) in self.dep_targets(pkg, target, profile).iter() {
            self.build_requirements(pkg, dep, profile, req);
        }

        match pkg.targets().iter().find(|t| t.is_custom_build()) {
            Some(custom_build) => {
                let profile = self.build_script_profile(pkg.package_id());
                self.build_requirements(pkg, custom_build, profile,
                                        Platform::Plugin);
            }
            None => {}
        }
    }

    pub fn get_requirement(&self, pkg: &'a Package,
                           target: &'a Target) -> Platform {
        let default = if target.for_host() {
            Platform::Plugin
        } else {
            Platform::Target
        };
        self.requirements.get(&(pkg.package_id(), target.name()))
            .map(|a| *a).unwrap_or(default)
    }

    /// Returns the appropriate directory layout for either a plugin or not.
    pub fn layout(&self, pkg: &Package, kind: Kind) -> LayoutProxy {
        let primary = pkg.package_id() == self.resolve.root();
        match kind {
            Kind::Host => LayoutProxy::new(&self.host, primary),
            Kind::Target =>  LayoutProxy::new(self.target.as_ref()
                                                .unwrap_or(&self.host),
                                            primary),
        }
    }

    /// Returns the appropriate output directory for the specified package and
    /// target.
    pub fn out_dir(&self, pkg: &Package, kind: Kind, target: &Target) -> PathBuf {
        let out_dir = self.layout(pkg, kind);
        if target.is_custom_build() {
            out_dir.build(pkg)
        } else if target.is_example() {
            out_dir.examples().to_path_buf()
        } else {
            out_dir.root().to_path_buf()
        }
    }

    /// Return the (prefix, suffix) pair for dynamic libraries.
    ///
    /// If `plugin` is true, the pair corresponds to the host platform,
    /// otherwise it corresponds to the target platform.
    fn dylib(&self, kind: Kind) -> CargoResult<(&str, &str)> {
        let (triple, pair) = if kind == Kind::Host {
            (self.config.rustc_host(), &self.host_dylib)
        } else {
            (&self.target_triple[..], &self.target_dylib)
        };
        match *pair {
            None => return Err(human(format!("dylib outputs are not supported \
                                              for {}", triple))),
            Some((ref s1, ref s2)) => Ok((s1, s2)),
        }
    }

    /// Return the target triple which this context is targeting.
    pub fn target_triple(&self) -> &str {
        &self.target_triple
    }

    /// Get the metadata for a target in a specific profile
    pub fn target_metadata(&self, target: &Target, profile: &Profile)
                           -> Option<Metadata> {
        let metadata = target.metadata();
        if target.is_lib() && profile.test {
            // Libs and their tests are built in parallel, so we need to make
            // sure that their metadata is different.
            metadata.map(|m| m.clone()).map(|mut m| {
                m.mix(&"test");
                m
            })
        } else if target.is_bin() && profile.test {
            // Make sure that the name of this test executable doesn't
            // conflicts with a library that has the same name and is
            // being tested
            let mut metadata = self.resolve.root().generate_metadata();
            metadata.mix(&format!("bin-{}", target.name()));
            Some(metadata)
        } else {
            metadata.map(|m| m.clone())
        }
    }

    /// Returns the file stem for a given target/profile combo
    pub fn file_stem(&self, target: &Target, profile: &Profile) -> String {
        match self.target_metadata(target, profile) {
            Some(ref metadata) => format!("{}{}", target.crate_name(),
                                          metadata.extra_filename),
            None if target.allows_underscores() => target.name().to_string(),
            None => target.crate_name().to_string(),
        }
    }

    /// Return the filenames that the given target for the given profile will
    /// generate.
    pub fn target_filenames(&self, target: &Target, profile: &Profile)
                            -> CargoResult<Vec<String>> {
        let stem = self.file_stem(target, profile);
        let suffix = if target.for_host() {&self.host_exe} else {&self.target_exe};

        let mut ret = Vec::new();
        match *target.kind() {
            TargetKind::Example | TargetKind::Bin | TargetKind::CustomBuild |
            TargetKind::Bench | TargetKind::Test => {
                ret.push(format!("{}{}", stem, suffix));
            }
            TargetKind::Lib(..) if profile.test => {
                ret.push(format!("{}{}", stem, suffix));
            }
            TargetKind::Lib(ref libs) => {
                for lib in libs.iter() {
                    match *lib {
                        LibKind::Dylib => {
                            let plugin = target.for_host();
                            let kind = if plugin {Kind::Host} else {Kind::Target};
                            let (prefix, suffix) = try!(self.dylib(kind));
                            ret.push(format!("{}{}{}", prefix, stem, suffix));
                        }
                        LibKind::Lib |
                        LibKind::Rlib => ret.push(format!("lib{}.rlib", stem)),
                        LibKind::StaticLib => ret.push(format!("lib{}.a", stem)),
                    }
                }
            }
        }
        assert!(ret.len() > 0);
        return Ok(ret);
    }

    /// For a package, return all targets which are registered as dependencies
    /// for that package.
    pub fn dep_targets(&self, pkg: &Package, target: &Target,
                       profile: &Profile)
                       -> Vec<(&'a Package, &'a Target, &'a Profile)> {
        if profile.doc {
            return self.doc_deps(pkg, target);
        }
        let deps = match self.resolve.deps(pkg.package_id()) {
            None => return Vec::new(),
            Some(deps) => deps,
        };
        let mut ret = deps.map(|id| self.get_package(id)).filter(|dep| {
            let pkg_dep = pkg.dependencies().iter().find(|d| {
                d.name() == dep.name()
            }).unwrap();

            // If this target is a build command, then we only want build
            // dependencies, otherwise we want everything *other than* build
            // dependencies.
            let is_correct_dep = target.is_custom_build() == pkg_dep.is_build();

            // If this dependency is *not* a transitive dependency, then it
            // only applies to test/example targets
            let is_actual_dep = pkg_dep.is_transitive() ||
                                target.is_test() ||
                                target.is_example() ||
                                profile.test;

            is_correct_dep && is_actual_dep
        }).filter_map(|pkg| {
            pkg.targets().iter().find(|t| t.is_lib()).map(|t| {
                (pkg, t, self.lib_profile(pkg.package_id()))
            })
        }).collect::<Vec<_>>();

        // If a target isn't actually a build script itself, then it depends on
        // the build script if there is one.
        if target.is_custom_build() { return ret }
        let pkg = self.get_package(pkg.package_id());
        if let Some(t) = pkg.targets().iter().find(|t| t.is_custom_build()) {
            ret.push((pkg, t, self.build_script_profile(pkg.package_id())));
        }

        // If this target is a binary, test, example, etc, then it depends on
        // the library of the same package. The call to `resolve.deps` above
        // didn't include `pkg` in the return values, so we need to special case
        // it here and see if we need to push `(pkg, pkg_lib_target)`.
        if target.is_lib() { return ret }
        if let Some(t) = pkg.targets().iter().find(|t| t.linkable()) {
            ret.push((pkg, t, self.lib_profile(pkg.package_id())));
        }

        // Integration tests/benchmarks require binaries to be built
        if profile.test && (target.is_test() || target.is_bench()) {
            ret.extend(pkg.targets().iter().filter(|t| t.is_bin())
                          .map(|t| (pkg, t, self.lib_profile(pkg.package_id()))));
        }
        return ret
    }

    /// Returns the dependencies necessary to document a package
    fn doc_deps(&self, pkg: &Package, target: &Target)
                -> Vec<(&'a Package, &'a Target, &'a Profile)> {
        let pkg = self.get_package(pkg.package_id());
        let deps = self.resolve.deps(pkg.package_id()).into_iter();
        let deps = deps.flat_map(|a| a).map(|id| {
            self.get_package(id)
        }).filter(|dep| {
            pkg.dependencies().iter().find(|d| {
                d.name() == dep.name()
            }).unwrap().is_transitive()
        }).filter_map(|dep| {
            dep.targets().iter().find(|t| t.is_lib()).map(|t| (dep, t))
        });

        // To document a library, we depend on dependencies actually being
        // built. If we're documenting *all* libraries, then we also depend on
        // the documentation of the library being built.
        let mut ret = Vec::new();
        for (dep, lib) in deps {
            ret.push((dep, lib, self.lib_profile(dep.package_id())));
            if self.build_config.doc_all {
                ret.push((dep, lib, &self.profiles.doc));
            }
        }

        // Be sure to build/run the build script for documented libraries as
        if let Some(t) = pkg.targets().iter().find(|t| t.is_custom_build()) {
            ret.push((pkg, t, self.build_script_profile(pkg.package_id())));
        }

        // If we document a binary, we need the library available
        if target.is_bin() {
            if let Some(t) = pkg.targets().iter().find(|t| t.is_lib()) {
                ret.push((pkg, t, self.lib_profile(pkg.package_id())));
            }
        }
        return ret
    }

    /// Gets a package for the given package id.
    pub fn get_package(&self, id: &PackageId) -> &'a Package {
        self.package_set.iter()
            .find(|pkg| id == pkg.package_id())
            .expect("Should have found package")
    }

    /// Get the user-specified linker for a particular host or target
    pub fn linker(&self, kind: Kind) -> Option<&str> {
        self.target_config(kind).linker.as_ref().map(|s| &s[..])
    }

    /// Get the user-specified `ar` program for a particular host or target
    pub fn ar(&self, kind: Kind) -> Option<&str> {
        self.target_config(kind).ar.as_ref().map(|s| &s[..])
    }

    /// Get the target configuration for a particular host or target
    fn target_config(&self, kind: Kind) -> &TargetConfig {
        match kind {
            Kind::Host => &self.build_config.host,
            Kind::Target => &self.build_config.target,
        }
    }

    /// Number of jobs specified for this build
    pub fn jobs(&self) -> u32 { self.build_config.jobs }

    /// Requested (not actual) target for the build
    pub fn requested_target(&self) -> Option<&str> {
        self.build_config.requested_target.as_ref().map(|s| &s[..])
    }

    pub fn lib_profile(&self, _pkg: &PackageId) -> &'a Profile {
        if self.build_config.release {
            &self.profiles.release
        } else {
            &self.profiles.dev
        }
    }

    pub fn build_script_profile(&self, _pkg: &PackageId) -> &'a Profile {
        // TODO: should build scripts always be built with a dev
        //       profile? How is this controlled at the CLI layer?
        &self.profiles.dev
    }
}

impl Platform {
    pub fn combine(self, other: Platform) -> Platform {
        match (self, other) {
            (Platform::Target, Platform::Target) => Platform::Target,
            (Platform::Plugin, Platform::Plugin) => Platform::Plugin,
            _ => Platform::PluginAndTarget,
        }
    }

    pub fn includes(self, kind: Kind) -> bool {
        match (self, kind) {
            (Platform::PluginAndTarget, _) |
            (Platform::Target, Kind::Target) |
            (Platform::Plugin, Kind::Host) => true,
            _ => false,
        }
    }

    pub fn each_kind<F>(self, mut f: F) where F: FnMut(Kind) {
        match self {
            Platform::Target => f(Kind::Target),
            Platform::Plugin => f(Kind::Host),
            Platform::PluginAndTarget => { f(Kind::Target); f(Kind::Host); }
        }
    }
}
