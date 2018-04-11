//!
//! Cargo compile currently does the following steps:
//!
//! All configurations are already injected as environment variables via the
//! main cargo command
//!
//! 1. Read the manifest
//! 2. Shell out to `cargo-resolve` with a list of dependencies and sources as
//!    stdin
//!
//!    a. Shell out to `--do update` and `--do list` for each source
//!    b. Resolve dependencies and return a list of name/version/source
//!
//! 3. Shell out to `--do download` for each source
//! 4. Shell out to `--do get` for each source, and build up the list of paths
//!    to pass to rustc -L
//! 5. Call `cargo-rustc` with the results of the resolver zipped together with
//!    the results of the `get`
//!
//!    a. Topologically sort the dependencies
//!    b. Compile each dependency in order, passing in the -L's pointing at each
//!       previously compiled dependency
//!

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use core::{Package, Source, Target};
use core::{PackageId, PackageIdSpec, Profile, Profiles, TargetKind, Workspace};
use core::resolver::{Method, Resolve};
use ops::{self, BuildOutput, DefaultExecutor, Executor};
use util::config::Config;
use util::{profile, CargoResult, CargoResultExt};

/// Contains information about how a package should be compiled.
#[derive(Debug)]
pub struct CompileOptions<'a> {
    pub config: &'a Config,
    /// Number of concurrent jobs to use.
    pub jobs: Option<u32>,
    /// The target platform to compile for (example: `i686-unknown-linux-gnu`).
    pub target: Option<String>,
    /// Extra features to build for the root package
    pub features: Vec<String>,
    /// Flag whether all available features should be built for the root package
    pub all_features: bool,
    /// Flag if the default feature should be built for the root package
    pub no_default_features: bool,
    /// A set of packages to build.
    pub spec: Packages,
    /// Filter to apply to the root package to select which targets will be
    /// built.
    pub filter: CompileFilter,
    /// Whether this is a release build or not
    pub release: bool,
    /// Mode for this compile.
    pub mode: CompileMode,
    /// `--error_format` flag for the compiler.
    pub message_format: MessageFormat,
    /// Extra arguments to be passed to rustdoc (for main crate and dependencies)
    pub target_rustdoc_args: Option<Vec<String>>,
    /// The specified target will be compiled with all the available arguments,
    /// note that this only accounts for the *final* invocation of rustc
    pub target_rustc_args: Option<Vec<String>>,
    /// The directory to copy final artifacts to. Note that even if `out_dir` is
    /// set, a copy of artifacts still could be found a `target/(debug\release)`
    /// as usual.
    // Note that, although the cmd-line flag name is `out-dir`, in code we use
    // `export_dir`, to avoid confusion with out dir at `target/debug/deps`.
    pub export_dir: Option<PathBuf>,
}

impl<'a> CompileOptions<'a> {
    pub fn default(config: &'a Config, mode: CompileMode) -> CompileOptions<'a> {
        CompileOptions {
            config,
            jobs: None,
            target: None,
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
            spec: ops::Packages::Packages(Vec::new()),
            mode,
            release: false,
            filter: CompileFilter::Default {
                required_features_filterable: false,
            },
            message_format: MessageFormat::Human,
            target_rustdoc_args: None,
            target_rustc_args: None,
            export_dir: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CompileMode {
    Test,
    Build,
    Check { test: bool },
    Bench,
    Doc { deps: bool },
    Doctest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageFormat {
    Human,
    Json,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Packages {
    Default,
    All,
    OptOut(Vec<String>),
    Packages(Vec<String>),
}

impl Packages {
    pub fn from_flags(all: bool, exclude: Vec<String>, package: Vec<String>) -> CargoResult<Self> {
        Ok(match (all, exclude.len(), package.len()) {
            (false, 0, 0) => Packages::Default,
            (false, 0, _) => Packages::Packages(package),
            (false, _, _) => bail!("--exclude can only be used together with --all"),
            (true, 0, _) => Packages::All,
            (true, _, _) => Packages::OptOut(exclude),
        })
    }

    pub fn into_package_id_specs(&self, ws: &Workspace) -> CargoResult<Vec<PackageIdSpec>> {
        let specs = match *self {
            Packages::All => ws.members()
                .map(Package::package_id)
                .map(PackageIdSpec::from_package_id)
                .collect(),
            Packages::OptOut(ref opt_out) => ws.members()
                .map(Package::package_id)
                .map(PackageIdSpec::from_package_id)
                .filter(|p| opt_out.iter().position(|x| *x == p.name()).is_none())
                .collect(),
            Packages::Packages(ref packages) if packages.is_empty() => ws.current_opt()
                .map(Package::package_id)
                .map(PackageIdSpec::from_package_id)
                .into_iter()
                .collect(),
            Packages::Packages(ref packages) => packages
                .iter()
                .map(|p| PackageIdSpec::parse(p))
                .collect::<CargoResult<Vec<_>>>()?,
            Packages::Default => ws.default_members()
                .map(Package::package_id)
                .map(PackageIdSpec::from_package_id)
                .collect(),
        };
        if specs.is_empty() {
            if ws.is_virtual() {
                bail!(
                    "manifest path `{}` contains no package: The manifest is virtual, \
                     and the workspace has no members.",
                    ws.root().display()
                )
            }
            bail!("no packages to compile")
        }
        Ok(specs)
    }
}

#[derive(Debug)]
pub enum FilterRule {
    All,
    Just(Vec<String>),
}

#[derive(Debug)]
pub enum CompileFilter {
    Default {
        /// Flag whether targets can be safely skipped when required-features are not satisfied.
        required_features_filterable: bool,
    },
    Only {
        all_targets: bool,
        lib: bool,
        bins: FilterRule,
        examples: FilterRule,
        tests: FilterRule,
        benches: FilterRule,
    },
}

pub fn compile<'a>(
    ws: &Workspace<'a>,
    options: &CompileOptions<'a>,
) -> CargoResult<ops::Compilation<'a>> {
    compile_with_exec(ws, options, Arc::new(DefaultExecutor))
}

pub fn compile_with_exec<'a>(
    ws: &Workspace<'a>,
    options: &CompileOptions<'a>,
    exec: Arc<Executor>,
) -> CargoResult<ops::Compilation<'a>> {
    for member in ws.members() {
        for warning in member.manifest().warnings().iter() {
            if warning.is_critical {
                let err = format_err!("{}", warning.message);
                let cx = format_err!(
                    "failed to parse manifest at `{}`",
                    member.manifest_path().display()
                );
                return Err(err.context(cx).into());
            } else {
                options.config.shell().warn(&warning.message)?
            }
        }
    }
    compile_ws(ws, None, options, exec)
}

pub fn compile_ws<'a>(
    ws: &Workspace<'a>,
    source: Option<Box<Source + 'a>>,
    options: &CompileOptions<'a>,
    exec: Arc<Executor>,
) -> CargoResult<ops::Compilation<'a>> {
    let CompileOptions {
        config,
        jobs,
        ref target,
        ref spec,
        ref features,
        all_features,
        no_default_features,
        release,
        mode,
        message_format,
        ref filter,
        ref target_rustdoc_args,
        ref target_rustc_args,
        ref export_dir,
    } = *options;

    let target = match target {
        &Some(ref target) if target.ends_with(".json") => {
            let path = Path::new(target)
                .canonicalize()
                .chain_err(|| format_err!("Target path {:?} is not a valid file", target))?;
            Some(path.into_os_string()
                .into_string()
                .map_err(|_| format_err!("Target path is not valid unicode"))?)
        }
        other => other.clone(),
    };

    if jobs == Some(0) {
        bail!("jobs must be at least 1")
    }

    let mut build_config = scrape_build_config(config, jobs, target)?;
    build_config.release = release;
    build_config.test = mode == CompileMode::Test || mode == CompileMode::Bench;
    build_config.json_messages = message_format == MessageFormat::Json;
    if let CompileMode::Doc { deps } = mode {
        build_config.doc_all = deps;
    }

    let profiles = ws.profiles();

    let specs = spec.into_package_id_specs(ws)?;
    let features = Method::split_features(features);
    let method = Method::Required {
        dev_deps: ws.require_optional_deps() || filter.need_dev_deps(mode),
        features: &features,
        all_features,
        uses_default_features: !no_default_features,
    };
    let resolve = ops::resolve_ws_with_method(ws, source, method, &specs)?;
    let (packages, resolve_with_overrides) = resolve;

    let to_builds = specs
        .iter()
        .map(|p| {
            let pkgid = p.query(resolve_with_overrides.iter())?;
            let p = packages.get(pkgid)?;
            p.manifest().print_teapot(ws.config());
            Ok(p)
        })
        .collect::<CargoResult<Vec<_>>>()?;

    let mut general_targets = Vec::new();
    let mut package_targets = Vec::new();

    match (target_rustc_args, target_rustdoc_args) {
        (&Some(..), _) | (_, &Some(..)) if to_builds.len() != 1 => {
            panic!("`rustc` and `rustdoc` should not accept multiple `-p` flags")
        }
        (&Some(ref args), _) => {
            let all_features =
                resolve_all_features(&resolve_with_overrides, to_builds[0].package_id());
            let targets =
                generate_targets(to_builds[0], profiles, mode, filter, &all_features, release)?;
            if targets.len() == 1 {
                let (target, profile) = targets[0];
                let mut profile = profile.clone();
                profile.rustc_args = Some(args.to_vec());
                general_targets.push((target, profile));
            } else {
                bail!(
                    "extra arguments to `rustc` can only be passed to one \
                     target, consider filtering\nthe package by passing \
                     e.g. `--lib` or `--bin NAME` to specify a single target"
                )
            }
        }
        (&None, &Some(ref args)) => {
            let all_features =
                resolve_all_features(&resolve_with_overrides, to_builds[0].package_id());
            let targets =
                generate_targets(to_builds[0], profiles, mode, filter, &all_features, release)?;
            if targets.len() == 1 {
                let (target, profile) = targets[0];
                let mut profile = profile.clone();
                profile.rustdoc_args = Some(args.to_vec());
                general_targets.push((target, profile));
            } else {
                bail!(
                    "extra arguments to `rustdoc` can only be passed to one \
                     target, consider filtering\nthe package by passing e.g. \
                     `--lib` or `--bin NAME` to specify a single target"
                )
            }
        }
        (&None, &None) => for &to_build in to_builds.iter() {
            let all_features = resolve_all_features(&resolve_with_overrides, to_build.package_id());
            let targets =
                generate_targets(to_build, profiles, mode, filter, &all_features, release)?;
            package_targets.push((to_build, targets));
        },
    };

    for &(target, ref profile) in &general_targets {
        for &to_build in to_builds.iter() {
            package_targets.push((to_build, vec![(target, profile)]));
        }
    }
    let mut ret = {
        let _p = profile::start("compiling");
        ops::compile_targets(
            ws,
            &package_targets,
            &packages,
            &resolve_with_overrides,
            config,
            build_config,
            profiles,
            export_dir.clone(),
            &exec,
        )?
    };

    ret.to_doc_test = to_builds.into_iter().cloned().collect();

    return Ok(ret);

    fn resolve_all_features(
        resolve_with_overrides: &Resolve,
        package_id: &PackageId,
    ) -> HashSet<String> {
        let mut features = resolve_with_overrides.features(package_id).clone();

        // Include features enabled for use by dependencies so targets can also use them with the
        // required-features field when deciding whether to be built or skipped.
        let deps = resolve_with_overrides.deps(package_id);
        for dep in deps {
            for feature in resolve_with_overrides.features(dep) {
                features.insert(dep.name().to_string() + "/" + feature);
            }
        }

        features
    }
}

impl FilterRule {
    pub fn new(targets: Vec<String>, all: bool) -> FilterRule {
        if all {
            FilterRule::All
        } else {
            FilterRule::Just(targets)
        }
    }

    fn matches(&self, target: &Target) -> bool {
        match *self {
            FilterRule::All => true,
            FilterRule::Just(ref targets) => targets.iter().any(|x| *x == target.name()),
        }
    }

    fn is_specific(&self) -> bool {
        match *self {
            FilterRule::All => true,
            FilterRule::Just(ref targets) => !targets.is_empty(),
        }
    }

    pub fn try_collect(&self) -> Option<Vec<String>> {
        match *self {
            FilterRule::All => None,
            FilterRule::Just(ref targets) => Some(targets.clone()),
        }
    }
}

impl CompileFilter {
    pub fn new(
        lib_only: bool,
        bins: Vec<String>,
        all_bins: bool,
        tsts: Vec<String>,
        all_tsts: bool,
        exms: Vec<String>,
        all_exms: bool,
        bens: Vec<String>,
        all_bens: bool,
        all_targets: bool,
    ) -> CompileFilter {
        let rule_bins = FilterRule::new(bins, all_bins);
        let rule_tsts = FilterRule::new(tsts, all_tsts);
        let rule_exms = FilterRule::new(exms, all_exms);
        let rule_bens = FilterRule::new(bens, all_bens);

        if all_targets {
            CompileFilter::Only {
                all_targets: true,
                lib: true,
                bins: FilterRule::All,
                examples: FilterRule::All,
                benches: FilterRule::All,
                tests: FilterRule::All,
            }
        } else if lib_only || rule_bins.is_specific() || rule_tsts.is_specific()
            || rule_exms.is_specific() || rule_bens.is_specific()
        {
            CompileFilter::Only {
                all_targets: false,
                lib: lib_only,
                bins: rule_bins,
                examples: rule_exms,
                benches: rule_bens,
                tests: rule_tsts,
            }
        } else {
            CompileFilter::Default {
                required_features_filterable: true,
            }
        }
    }

    pub fn need_dev_deps(&self, mode: CompileMode) -> bool {
        match mode {
            CompileMode::Test | CompileMode::Doctest | CompileMode::Bench => true,
            CompileMode::Build | CompileMode::Doc { .. } | CompileMode::Check { .. } => match *self
            {
                CompileFilter::Default { .. } => false,
                CompileFilter::Only {
                    ref examples,
                    ref tests,
                    ref benches,
                    ..
                } => examples.is_specific() || tests.is_specific() || benches.is_specific(),
            },
        }
    }

    // this selects targets for "cargo run". for logic to select targets for
    // other subcommands, see generate_targets and generate_default_targets
    pub fn target_run(&self, target: &Target) -> bool {
        match *self {
            CompileFilter::Default { .. } => true,
            CompileFilter::Only {
                lib,
                ref bins,
                ref examples,
                ref tests,
                ref benches,
                ..
            } => {
                let rule = match *target.kind() {
                    TargetKind::Bin => bins,
                    TargetKind::Test => tests,
                    TargetKind::Bench => benches,
                    TargetKind::ExampleBin | TargetKind::ExampleLib(..) => examples,
                    TargetKind::Lib(..) => return lib,
                    TargetKind::CustomBuild => return false,
                };
                rule.matches(target)
            }
        }
    }

    pub fn is_specific(&self) -> bool {
        match *self {
            CompileFilter::Default { .. } => false,
            CompileFilter::Only { .. } => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BuildProposal<'a> {
    target: &'a Target,
    profile: &'a Profile,
    required: bool,
}

fn generate_default_targets<'a>(
    mode: CompileMode,
    targets: &'a [Target],
    profile: &'a Profile,
    dep: &'a Profile,
    required_features_filterable: bool,
) -> Vec<BuildProposal<'a>> {
    match mode {
        CompileMode::Bench => targets
            .iter()
            .filter(|t| t.benched())
            .map(|t| BuildProposal {
                target: t,
                profile,
                required: !required_features_filterable,
            })
            .collect::<Vec<_>>(),
        CompileMode::Test => {
            let mut base = targets
                .iter()
                .filter(|t| t.tested())
                .map(|t| BuildProposal {
                    target: t,
                    profile: if t.is_example() { dep } else { profile },
                    required: !required_features_filterable,
                })
                .collect::<Vec<_>>();

            // Always compile the library if we're testing everything as
            // it'll be needed for doctests
            if let Some(t) = targets.iter().find(|t| t.is_lib()) {
                if t.doctested() {
                    base.push(BuildProposal {
                        target: t,
                        profile: dep,
                        required: !required_features_filterable,
                    });
                }
            }
            base
        }
        CompileMode::Build | CompileMode::Check { .. } => targets
            .iter()
            .filter(|t| t.is_bin() || t.is_lib())
            .map(|t| BuildProposal {
                target: t,
                profile,
                required: !required_features_filterable,
            })
            .collect(),
        CompileMode::Doc { .. } => targets
            .iter()
            .filter(|t| {
                t.documented()
                    && (!t.is_bin() || !targets.iter().any(|l| l.is_lib() && l.name() == t.name()))
            })
            .map(|t| BuildProposal {
                target: t,
                profile,
                required: !required_features_filterable,
            })
            .collect(),
        CompileMode::Doctest => {
            if let Some(t) = targets.iter().find(|t| t.is_lib()) {
                if t.doctested() {
                    return vec![
                        BuildProposal {
                            target: t,
                            profile,
                            required: !required_features_filterable,
                        },
                    ];
                }
            }

            Vec::new()
        }
    }
}

/// Given a filter rule and some context, propose a list of targets
fn propose_indicated_targets<'a>(
    pkg: &'a Package,
    rule: &FilterRule,
    desc: &'static str,
    is_expected_kind: fn(&Target) -> bool,
    profile: &'a Profile,
) -> CargoResult<Vec<BuildProposal<'a>>> {
    match *rule {
        FilterRule::All => {
            let result = pkg.targets()
                .iter()
                .filter(|t| is_expected_kind(t))
                .map(|t| BuildProposal {
                    target: t,
                    profile,
                    required: false,
                });
            Ok(result.collect())
        }
        FilterRule::Just(ref names) => {
            let mut targets = Vec::new();
            for name in names {
                let target = pkg.targets()
                    .iter()
                    .find(|t| t.name() == *name && is_expected_kind(t));
                let t = match target {
                    Some(t) => t,
                    None => {
                        let suggestion = pkg.find_closest_target(name, is_expected_kind);
                        match suggestion {
                            Some(s) => {
                                let suggested_name = s.name();
                                bail!(
                                    "no {} target named `{}`\n\nDid you mean `{}`?",
                                    desc,
                                    name,
                                    suggested_name
                                )
                            }
                            None => bail!("no {} target named `{}`", desc, name),
                        }
                    }
                };
                debug!("found {} `{}`", desc, name);
                targets.push(BuildProposal {
                    target: t,
                    profile,
                    required: true,
                });
            }
            Ok(targets)
        }
    }
}

/// Collect the targets that are libraries or have all required features available.
fn filter_compatible_targets<'a>(
    mut proposals: Vec<BuildProposal<'a>>,
    features: &HashSet<String>,
) -> CargoResult<Vec<(&'a Target, &'a Profile)>> {
    let mut compatible = Vec::with_capacity(proposals.len());
    for proposal in proposals.drain(..) {
        let unavailable_features = match proposal.target.required_features() {
            Some(rf) => rf.iter().filter(|f| !features.contains(*f)).collect(),
            None => Vec::new(),
        };
        if proposal.target.is_lib() || unavailable_features.is_empty() {
            compatible.push((proposal.target, proposal.profile));
        } else if proposal.required {
            let required_features = proposal.target.required_features().unwrap();
            let quoted_required_features: Vec<String> = required_features
                .iter()
                .map(|s| format!("`{}`", s))
                .collect();
            bail!(
                "target `{}` requires the features: {}\n\
                 Consider enabling them by passing e.g. `--features=\"{}\"`",
                proposal.target.name(),
                quoted_required_features.join(", "),
                required_features.join(" ")
            );
        }
    }
    Ok(compatible)
}

/// Given the configuration for a build, this function will generate all
/// target/profile combinations needed to be built.
fn generate_targets<'a>(
    pkg: &'a Package,
    profiles: &'a Profiles,
    mode: CompileMode,
    filter: &CompileFilter,
    features: &HashSet<String>,
    release: bool,
) -> CargoResult<Vec<(&'a Target, &'a Profile)>> {
    let build = if release {
        &profiles.release
    } else {
        &profiles.dev
    };
    let test = if release {
        &profiles.bench
    } else {
        &profiles.test
    };
    let profile = match mode {
        CompileMode::Test => test,
        CompileMode::Bench => &profiles.bench,
        CompileMode::Build => build,
        CompileMode::Check { test: false } => &profiles.check,
        CompileMode::Check { test: true } => &profiles.check_test,
        CompileMode::Doc { .. } => &profiles.doc,
        CompileMode::Doctest => &profiles.doctest,
    };

    let test_profile = if profile.check {
        &profiles.check_test
    } else if mode == CompileMode::Build {
        test
    } else {
        profile
    };

    let bench_profile = if profile.check {
        &profiles.check_test
    } else if mode == CompileMode::Build {
        &profiles.bench
    } else {
        profile
    };

    let targets = match *filter {
        CompileFilter::Default {
            required_features_filterable,
        } => {
            let deps = if release {
                &profiles.bench_deps
            } else {
                &profiles.test_deps
            };
            generate_default_targets(
                mode,
                pkg.targets(),
                profile,
                deps,
                required_features_filterable,
            )
        }
        CompileFilter::Only {
            all_targets,
            lib,
            ref bins,
            ref examples,
            ref tests,
            ref benches,
        } => {
            let mut targets = Vec::new();

            if lib {
                if let Some(t) = pkg.targets().iter().find(|t| t.is_lib()) {
                    targets.push(BuildProposal {
                        target: t,
                        profile,
                        required: true,
                    });
                } else if !all_targets {
                    bail!("no library targets found")
                }
            }
            targets.append(&mut propose_indicated_targets(
                pkg,
                bins,
                "bin",
                Target::is_bin,
                profile,
            )?);
            targets.append(&mut propose_indicated_targets(
                pkg,
                examples,
                "example",
                Target::is_example,
                profile,
            )?);
            // If --tests was specified, add all targets that would be
            // generated by `cargo test`.
            let test_filter = match *tests {
                FilterRule::All => Target::tested,
                FilterRule::Just(_) => Target::is_test,
            };
            targets.append(&mut propose_indicated_targets(
                pkg,
                tests,
                "test",
                test_filter,
                test_profile,
            )?);
            // If --benches was specified, add all targets that would be
            // generated by `cargo bench`.
            let bench_filter = match *benches {
                FilterRule::All => Target::benched,
                FilterRule::Just(_) => Target::is_bench,
            };
            targets.append(&mut propose_indicated_targets(
                pkg,
                benches,
                "bench",
                bench_filter,
                bench_profile,
            )?);
            targets
        }
    };

    filter_compatible_targets(targets, features)
}

/// Parse all config files to learn about build configuration. Currently
/// configured options are:
///
/// * build.jobs
/// * build.target
/// * target.$target.ar
/// * target.$target.linker
/// * target.$target.libfoo.metadata
fn scrape_build_config(
    config: &Config,
    jobs: Option<u32>,
    target: Option<String>,
) -> CargoResult<ops::BuildConfig> {
    if jobs.is_some() && config.jobserver_from_env().is_some() {
        config.shell().warn(
            "a `-j` argument was passed to Cargo but Cargo is \
             also configured with an external jobserver in \
             its environment, ignoring the `-j` parameter",
        )?;
    }
    let cfg_jobs = match config.get_i64("build.jobs")? {
        Some(v) => {
            if v.val <= 0 {
                bail!(
                    "build.jobs must be positive, but found {} in {}",
                    v.val,
                    v.definition
                )
            } else if v.val >= i64::from(u32::max_value()) {
                bail!(
                    "build.jobs is too large: found {} in {}",
                    v.val,
                    v.definition
                )
            } else {
                Some(v.val as u32)
            }
        }
        None => None,
    };
    let jobs = jobs.or(cfg_jobs).unwrap_or(::num_cpus::get() as u32);
    let cfg_target = config.get_string("build.target")?.map(|s| s.val);
    let target = target.or(cfg_target);
    let mut base = ops::BuildConfig::new(&config.rustc()?.host, &target)?;
    base.jobs = jobs;
    base.host = scrape_target_config(config, &base.host_triple)?;
    base.target = match target.as_ref() {
        Some(triple) => scrape_target_config(config, triple)?,
        None => base.host.clone(),
    };
    Ok(base)
}

fn scrape_target_config(config: &Config, triple: &str) -> CargoResult<ops::TargetConfig> {
    let key = format!("target.{}", triple);
    let mut ret = ops::TargetConfig {
        ar: config.get_path(&format!("{}.ar", key))?.map(|v| v.val),
        linker: config.get_path(&format!("{}.linker", key))?.map(|v| v.val),
        overrides: HashMap::new(),
    };
    let table = match config.get_table(&key)? {
        Some(table) => table.val,
        None => return Ok(ret),
    };
    for (lib_name, value) in table {
        match lib_name.as_str() {
            "ar" | "linker" | "runner" | "rustflags" => continue,
            _ => {}
        }

        let mut output = BuildOutput {
            library_paths: Vec::new(),
            library_links: Vec::new(),
            cfgs: Vec::new(),
            env: Vec::new(),
            metadata: Vec::new(),
            rerun_if_changed: Vec::new(),
            rerun_if_env_changed: Vec::new(),
            warnings: Vec::new(),
        };
        // We require deterministic order of evaluation, so we must sort the pairs by key first.
        let mut pairs = Vec::new();
        for (k, value) in value.table(&lib_name)?.0 {
            pairs.push((k, value));
        }
        pairs.sort_by_key(|p| p.0);
        for (k, value) in pairs {
            let key = format!("{}.{}", key, k);
            match &k[..] {
                "rustc-flags" => {
                    let (flags, definition) = value.string(k)?;
                    let whence = format!("in `{}` (in {})", key, definition.display());
                    let (paths, links) = BuildOutput::parse_rustc_flags(flags, &whence)?;
                    output.library_paths.extend(paths);
                    output.library_links.extend(links);
                }
                "rustc-link-lib" => {
                    let list = value.list(k)?;
                    output
                        .library_links
                        .extend(list.iter().map(|v| v.0.clone()));
                }
                "rustc-link-search" => {
                    let list = value.list(k)?;
                    output
                        .library_paths
                        .extend(list.iter().map(|v| PathBuf::from(&v.0)));
                }
                "rustc-cfg" => {
                    let list = value.list(k)?;
                    output.cfgs.extend(list.iter().map(|v| v.0.clone()));
                }
                "rustc-env" => for (name, val) in value.table(k)?.0 {
                    let val = val.string(name)?.0;
                    output.env.push((name.clone(), val.to_string()));
                },
                "warning" | "rerun-if-changed" | "rerun-if-env-changed" => {
                    bail!("`{}` is not supported in build script overrides", k);
                }
                _ => {
                    let val = value.string(k)?.0;
                    output.metadata.push((k.clone(), val.to_string()));
                }
            }
        }
        ret.overrides.insert(lib_name, output);
    }

    Ok(ret)
}
