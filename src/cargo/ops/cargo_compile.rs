//! # The Cargo "compile" operation
//!
//! This module contains the entry point for starting the compilation process
//! for commands like `build`, `test`, `doc`, `rustc`, etc.
//!
//! The [`compile`] function will do all the work to compile a workspace. A
//! rough outline is:
//!
//! - Resolve the dependency graph (see [`ops::resolve`]).
//! - Download any packages needed (see [`PackageSet`]).
//! - Generate a list of top-level "units" of work for the targets the user
//!   requested on the command-line. Each [`Unit`] corresponds to a compiler
//!   invocation. This is done in this module ([`generate_targets`]).
//! - Build the graph of `Unit` dependencies (see [`unit_dependencies`]).
//! - Create a [`Context`] which will perform the following steps:
//!     - Prepare the `target` directory (see [`Layout`]).
//!     - Create a job queue (see `JobQueue`). The queue checks the
//!       fingerprint of each `Unit` to determine if it should run or be
//!       skipped.
//!     - Execute the queue. Each leaf in the queue's dependency graph is
//!       executed, and then removed from the graph when finished. This
//!       repeats until the queue is empty.
//!
//! **Note**: "target" inside this module generally refers to ["Cargo Target"],
//! which corresponds to artifact that will be built in a package. Not to be
//! confused with target-triple or target architecture.
//!
//! [`unit_dependencies`]: crate::core::compiler::unit_dependencies
//! [`Layout`]: crate::core::compiler::Layout
//! ["Cargo Target"]: https://doc.rust-lang.org/nightly/cargo/reference/cargo-targets.html

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::core::compiler::unit_dependencies::{build_unit_dependencies, IsArtifact};
use crate::core::compiler::unit_graph::{self, UnitDep, UnitGraph};
use crate::core::compiler::{standard_lib, CrateType, TargetInfo};
use crate::core::compiler::{BuildConfig, BuildContext, Compilation, Context};
use crate::core::compiler::{CompileKind, CompileMode, CompileTarget, RustcTargetData, Unit};
use crate::core::compiler::{DefaultExecutor, Executor, UnitInterner};
use crate::core::profiles::{Profiles, UnitFor};
use crate::core::resolver::features::{self, CliFeatures, FeaturesFor};
use crate::core::resolver::{HasDevUnits, Resolve};
use crate::core::{FeatureValue, Package, PackageSet, Shell, Summary, Target};
use crate::core::{PackageId, SourceId, TargetKind, Workspace};
use crate::drop_println;
use crate::ops;
use crate::ops::resolve::WorkspaceResolve;
use crate::util::config::Config;
use crate::util::interning::InternedString;
use crate::util::restricted_names::is_glob_pattern;
use crate::util::{closest_msg, profile, CargoResult, StableHasher};

mod compile_filter;
pub use compile_filter::{CompileFilter, FilterRule, LibRule};

mod packages;
use packages::build_glob;
pub use packages::Packages;

/// Contains information about how a package should be compiled.
///
/// Note on distinction between `CompileOptions` and [`BuildConfig`]:
/// `BuildConfig` contains values that need to be retained after
/// [`BuildContext`] is created. The other fields are no longer necessary. Think
/// of it as `CompileOptions` are high-level settings requested on the
/// command-line, and `BuildConfig` are low-level settings for actually
/// driving `rustc`.
#[derive(Debug)]
pub struct CompileOptions {
    /// Configuration information for a rustc build
    pub build_config: BuildConfig,
    /// Feature flags requested by the user.
    pub cli_features: CliFeatures,
    /// A set of packages to build.
    pub spec: Packages,
    /// Filter to apply to the root package to select which targets will be
    /// built.
    pub filter: CompileFilter,
    /// Extra arguments to be passed to rustdoc (single target only)
    pub target_rustdoc_args: Option<Vec<String>>,
    /// The specified target will be compiled with all the available arguments,
    /// note that this only accounts for the *final* invocation of rustc
    pub target_rustc_args: Option<Vec<String>>,
    /// Crate types to be passed to rustc (single target only)
    pub target_rustc_crate_types: Option<Vec<String>>,
    /// Extra arguments passed to all selected targets for rustdoc.
    pub local_rustdoc_args: Option<Vec<String>>,
    /// Whether the `--document-private-items` flags was specified and should
    /// be forwarded to `rustdoc`.
    pub rustdoc_document_private_items: bool,
    /// Whether the build process should check the minimum Rust version
    /// defined in the cargo metadata for a crate.
    pub honor_rust_version: bool,
}

impl CompileOptions {
    pub fn new(config: &Config, mode: CompileMode) -> CargoResult<CompileOptions> {
        let jobs = None;
        let keep_going = false;
        Ok(CompileOptions {
            build_config: BuildConfig::new(config, jobs, keep_going, &[], mode)?,
            cli_features: CliFeatures::new_all(false),
            spec: ops::Packages::Packages(Vec::new()),
            filter: CompileFilter::Default {
                required_features_filterable: false,
            },
            target_rustdoc_args: None,
            target_rustc_args: None,
            target_rustc_crate_types: None,
            local_rustdoc_args: None,
            rustdoc_document_private_items: false,
            honor_rust_version: true,
        })
    }
}

/// Compiles!
///
/// This uses the [`DefaultExecutor`]. To use a custom [`Executor`], see [`compile_with_exec`].
pub fn compile<'a>(ws: &Workspace<'a>, options: &CompileOptions) -> CargoResult<Compilation<'a>> {
    let exec: Arc<dyn Executor> = Arc::new(DefaultExecutor);
    compile_with_exec(ws, options, &exec)
}

/// Like [`compile`] but allows specifying a custom [`Executor`]
/// that will be able to intercept build calls and add custom logic.
///
/// [`compile`] uses [`DefaultExecutor`] which just passes calls through.
pub fn compile_with_exec<'a>(
    ws: &Workspace<'a>,
    options: &CompileOptions,
    exec: &Arc<dyn Executor>,
) -> CargoResult<Compilation<'a>> {
    ws.emit_warnings()?;
    compile_ws(ws, options, exec)
}

/// Like [`compile_with_exec`] but without warnings from manifest parsing.
pub fn compile_ws<'a>(
    ws: &Workspace<'a>,
    options: &CompileOptions,
    exec: &Arc<dyn Executor>,
) -> CargoResult<Compilation<'a>> {
    let interner = UnitInterner::new();
    let bcx = create_bcx(ws, options, &interner)?;
    if options.build_config.unit_graph {
        unit_graph::emit_serialized_unit_graph(&bcx.roots, &bcx.unit_graph, ws.config())?;
        return Compilation::new(&bcx);
    }
    let _p = profile::start("compiling");
    let cx = Context::new(&bcx)?;
    cx.compile(exec)
}

/// Executes `rustc --print <VALUE>`.
///
/// * `print_opt_value` is the VALUE passed through.
pub fn print<'a>(
    ws: &Workspace<'a>,
    options: &CompileOptions,
    print_opt_value: &str,
) -> CargoResult<()> {
    let CompileOptions {
        ref build_config,
        ref target_rustc_args,
        ..
    } = *options;
    let config = ws.config();
    let rustc = config.load_global_rustc(Some(ws))?;
    for (index, kind) in build_config.requested_kinds.iter().enumerate() {
        if index != 0 {
            drop_println!(config);
        }
        let target_info = TargetInfo::new(config, &build_config.requested_kinds, &rustc, *kind)?;
        let mut process = rustc.process();
        process.args(&target_info.rustflags);
        if let Some(args) = target_rustc_args {
            process.args(args);
        }
        if let CompileKind::Target(t) = kind {
            process.arg("--target").arg(t.short_name());
        }
        process.arg("--print").arg(print_opt_value);
        process.exec()?;
    }
    Ok(())
}

/// Prepares all required information for the actual compilation.
///
/// For how it works and what data it collects,
/// please see the [module-level documentation](self).
pub fn create_bcx<'a, 'cfg>(
    ws: &'a Workspace<'cfg>,
    options: &'a CompileOptions,
    interner: &'a UnitInterner,
) -> CargoResult<BuildContext<'a, 'cfg>> {
    let CompileOptions {
        ref build_config,
        ref spec,
        ref cli_features,
        ref filter,
        ref target_rustdoc_args,
        ref target_rustc_args,
        ref target_rustc_crate_types,
        ref local_rustdoc_args,
        rustdoc_document_private_items,
        honor_rust_version,
    } = *options;
    let config = ws.config();

    // Perform some pre-flight validation.
    match build_config.mode {
        CompileMode::Test
        | CompileMode::Build
        | CompileMode::Check { .. }
        | CompileMode::Bench
        | CompileMode::RunCustomBuild => {
            if std::env::var("RUST_FLAGS").is_ok() {
                config.shell().warn(
                    "Cargo does not read `RUST_FLAGS` environment variable. Did you mean `RUSTFLAGS`?",
                )?;
            }
        }
        CompileMode::Doc { .. } | CompileMode::Doctest | CompileMode::Docscrape => {
            if std::env::var("RUSTDOC_FLAGS").is_ok() {
                config.shell().warn(
                    "Cargo does not read `RUSTDOC_FLAGS` environment variable. Did you mean `RUSTDOCFLAGS`?"
                )?;
            }
        }
    }
    config.validate_term_config()?;

    let target_data = RustcTargetData::new(ws, &build_config.requested_kinds)?;

    let all_packages = &Packages::All;
    let rustdoc_scrape_examples = &config.cli_unstable().rustdoc_scrape_examples;
    let need_reverse_dependencies = rustdoc_scrape_examples.is_some();
    let full_specs = if need_reverse_dependencies {
        all_packages
    } else {
        spec
    };

    let resolve_specs = full_specs.to_package_id_specs(ws)?;
    let has_dev_units = if filter.need_dev_deps(build_config.mode) || need_reverse_dependencies {
        HasDevUnits::Yes
    } else {
        HasDevUnits::No
    };
    let resolve = ops::resolve_ws_with_opts(
        ws,
        &target_data,
        &build_config.requested_kinds,
        cli_features,
        &resolve_specs,
        has_dev_units,
        crate::core::resolver::features::ForceAllTargets::No,
    )?;
    let WorkspaceResolve {
        mut pkg_set,
        workspace_resolve,
        targeted_resolve: resolve,
        resolved_features,
    } = resolve;

    let std_resolve_features = if let Some(crates) = &config.cli_unstable().build_std {
        let (std_package_set, std_resolve, std_features) =
            standard_lib::resolve_std(ws, &target_data, &build_config, crates)?;
        pkg_set.add_set(std_package_set);
        Some((std_resolve, std_features))
    } else {
        None
    };

    // Find the packages in the resolver that the user wants to build (those
    // passed in with `-p` or the defaults from the workspace), and convert
    // Vec<PackageIdSpec> to a Vec<PackageId>.
    let specs = if need_reverse_dependencies {
        spec.to_package_id_specs(ws)?
    } else {
        resolve_specs.clone()
    };
    let to_build_ids = resolve.specs_to_ids(&specs)?;
    // Now get the `Package` for each `PackageId`. This may trigger a download
    // if the user specified `-p` for a dependency that is not downloaded.
    // Dependencies will be downloaded during build_unit_dependencies.
    let mut to_builds = pkg_set.get_many(to_build_ids)?;

    // The ordering here affects some error messages coming out of cargo, so
    // let's be test and CLI friendly by always printing in the same order if
    // there's an error.
    to_builds.sort_by_key(|p| p.package_id());

    for pkg in to_builds.iter() {
        pkg.manifest().print_teapot(config);

        if build_config.mode.is_any_test()
            && !ws.is_member(pkg)
            && pkg.dependencies().iter().any(|dep| !dep.is_transitive())
        {
            anyhow::bail!(
                "package `{}` cannot be tested because it requires dev-dependencies \
                 and is not a member of the workspace",
                pkg.name()
            );
        }
    }

    let (extra_args, extra_args_name) = match (target_rustc_args, target_rustdoc_args) {
        (&Some(ref args), _) => (Some(args.clone()), "rustc"),
        (_, &Some(ref args)) => (Some(args.clone()), "rustdoc"),
        _ => (None, ""),
    };

    if extra_args.is_some() && to_builds.len() != 1 {
        panic!(
            "`{}` should not accept multiple `-p` flags",
            extra_args_name
        );
    }

    let profiles = Profiles::new(ws, build_config.requested_profile)?;
    profiles.validate_packages(
        ws.profiles(),
        &mut config.shell(),
        workspace_resolve.as_ref().unwrap_or(&resolve),
    )?;

    // If `--target` has not been specified, then the unit graph is built
    // assuming `--target $HOST` was specified. See
    // `rebuild_unit_graph_shared` for more on why this is done.
    let explicit_host_kind = CompileKind::Target(CompileTarget::new(&target_data.rustc.host)?);
    let explicit_host_kinds: Vec<_> = build_config
        .requested_kinds
        .iter()
        .map(|kind| match kind {
            CompileKind::Host => explicit_host_kind,
            CompileKind::Target(t) => CompileKind::Target(*t),
        })
        .collect();

    // Passing `build_config.requested_kinds` instead of
    // `explicit_host_kinds` here so that `generate_targets` can do
    // its own special handling of `CompileKind::Host`. It will
    // internally replace the host kind by the `explicit_host_kind`
    // before setting as a unit.
    let mut units = generate_targets(
        ws,
        &to_builds,
        filter,
        &build_config.requested_kinds,
        explicit_host_kind,
        build_config.mode,
        &resolve,
        &workspace_resolve,
        &resolved_features,
        &pkg_set,
        &profiles,
        interner,
    )?;

    if let Some(args) = target_rustc_crate_types {
        override_rustc_crate_types(&mut units, args, interner)?;
    }

    let mut scrape_units = match rustdoc_scrape_examples {
        Some(arg) => {
            let filter = match arg.as_str() {
                "all" => CompileFilter::new_all_targets(),
                "examples" => CompileFilter::new(
                    LibRule::False,
                    FilterRule::none(),
                    FilterRule::none(),
                    FilterRule::All,
                    FilterRule::none(),
                ),
                _ => {
                    anyhow::bail!(
                        r#"-Z rustdoc-scrape-examples must take "all" or "examples" as an argument"#
                    )
                }
            };
            let to_build_ids = resolve.specs_to_ids(&resolve_specs)?;
            let to_builds = pkg_set.get_many(to_build_ids)?;
            let mode = CompileMode::Docscrape;

            generate_targets(
                ws,
                &to_builds,
                &filter,
                &build_config.requested_kinds,
                explicit_host_kind,
                mode,
                &resolve,
                &workspace_resolve,
                &resolved_features,
                &pkg_set,
                &profiles,
                interner,
            )?
            .into_iter()
            // Proc macros should not be scraped for functions, since they only export macros
            .filter(|unit| !unit.target.proc_macro())
            .collect::<Vec<_>>()
        }
        None => Vec::new(),
    };

    let std_roots = if let Some(crates) = standard_lib::std_crates(config, Some(&units)) {
        let (std_resolve, std_features) = std_resolve_features.as_ref().unwrap();
        standard_lib::generate_std_roots(
            &crates,
            std_resolve,
            std_features,
            &explicit_host_kinds,
            &pkg_set,
            interner,
            &profiles,
        )?
    } else {
        Default::default()
    };

    let mut unit_graph = build_unit_dependencies(
        ws,
        &pkg_set,
        &resolve,
        &resolved_features,
        std_resolve_features.as_ref(),
        &units,
        &scrape_units,
        &std_roots,
        build_config.mode,
        &target_data,
        &profiles,
        interner,
    )?;

    // TODO: In theory, Cargo should also dedupe the roots, but I'm uncertain
    // what heuristics to use in that case.
    if build_config.mode == (CompileMode::Doc { deps: true }) {
        remove_duplicate_doc(build_config, &units, &mut unit_graph);
    }

    if build_config
        .requested_kinds
        .iter()
        .any(CompileKind::is_host)
    {
        // Rebuild the unit graph, replacing the explicit host targets with
        // CompileKind::Host, merging any dependencies shared with build
        // dependencies.
        let new_graph = rebuild_unit_graph_shared(
            interner,
            unit_graph,
            &units,
            &scrape_units,
            explicit_host_kind,
        );
        // This would be nicer with destructuring assignment.
        units = new_graph.0;
        scrape_units = new_graph.1;
        unit_graph = new_graph.2;
    }

    let mut extra_compiler_args = HashMap::new();
    if let Some(args) = extra_args {
        if units.len() != 1 {
            anyhow::bail!(
                "extra arguments to `{}` can only be passed to one \
                 target, consider filtering\nthe package by passing, \
                 e.g., `--lib` or `--bin NAME` to specify a single target",
                extra_args_name
            );
        }
        extra_compiler_args.insert(units[0].clone(), args);
    }

    for unit in &units {
        if unit.mode.is_doc() || unit.mode.is_doc_test() {
            let mut extra_args = local_rustdoc_args.clone();

            // Add `--document-private-items` rustdoc flag if requested or if
            // the target is a binary. Binary crates get their private items
            // documented by default.
            if rustdoc_document_private_items || unit.target.is_bin() {
                let mut args = extra_args.take().unwrap_or_default();
                args.push("--document-private-items".into());
                if unit.target.is_bin() {
                    // This warning only makes sense if it's possible to document private items
                    // sometimes and ignore them at other times. But cargo consistently passes
                    // `--document-private-items`, so the warning isn't useful.
                    args.push("-Arustdoc::private-intra-doc-links".into());
                }
                extra_args = Some(args);
            }

            if let Some(args) = extra_args {
                extra_compiler_args
                    .entry(unit.clone())
                    .or_default()
                    .extend(args);
            }
        }
    }

    if honor_rust_version {
        // Remove any pre-release identifiers for easier comparison
        let current_version = &target_data.rustc.version;
        let untagged_version = semver::Version::new(
            current_version.major,
            current_version.minor,
            current_version.patch,
        );

        for unit in unit_graph.keys() {
            let version = match unit.pkg.rust_version() {
                Some(v) => v,
                None => continue,
            };

            let req = semver::VersionReq::parse(version).unwrap();
            if req.matches(&untagged_version) {
                continue;
            }

            let guidance = if ws.is_ephemeral() {
                if ws.ignore_lock() {
                    "Try re-running cargo install with `--locked`".to_string()
                } else {
                    String::new()
                }
            } else if !unit.is_local() {
                format!(
                    "Either upgrade to rustc {} or newer, or use\n\
                     cargo update -p {}@{} --precise ver\n\
                     where `ver` is the latest version of `{}` supporting rustc {}",
                    version,
                    unit.pkg.name(),
                    unit.pkg.version(),
                    unit.pkg.name(),
                    current_version,
                )
            } else {
                String::new()
            };

            anyhow::bail!(
                "package `{}` cannot be built because it requires rustc {} or newer, \
                 while the currently active rustc version is {}\n{}",
                unit.pkg,
                version,
                current_version,
                guidance,
            );
        }
    }

    let bcx = BuildContext::new(
        ws,
        pkg_set,
        build_config,
        profiles,
        extra_compiler_args,
        target_data,
        units,
        unit_graph,
        scrape_units,
    )?;

    Ok(bcx)
}

/// A proposed target.
///
/// Proposed targets are later filtered into actual `Unit`s based on whether or
/// not the target requires its features to be present.
#[derive(Debug)]
struct Proposal<'a> {
    pkg: &'a Package,
    target: &'a Target,
    /// Indicates whether or not all required features *must* be present. If
    /// false, and the features are not available, then it will be silently
    /// skipped. Generally, targets specified by name (`--bin foo`) are
    /// required, all others can be silently skipped if features are missing.
    requires_features: bool,
    mode: CompileMode,
}

/// Generates all the base targets for the packages the user has requested to
/// compile. Dependencies for these targets are computed later in `unit_dependencies`.
fn generate_targets(
    ws: &Workspace<'_>,
    packages: &[&Package],
    filter: &CompileFilter,
    requested_kinds: &[CompileKind],
    explicit_host_kind: CompileKind,
    mode: CompileMode,
    resolve: &Resolve,
    workspace_resolve: &Option<Resolve>,
    resolved_features: &features::ResolvedFeatures,
    package_set: &PackageSet<'_>,
    profiles: &Profiles,
    interner: &UnitInterner,
) -> CargoResult<Vec<Unit>> {
    let config = ws.config();
    // Helper for creating a list of `Unit` structures
    let new_unit = |units: &mut HashSet<Unit>,
                    pkg: &Package,
                    target: &Target,
                    initial_target_mode: CompileMode| {
        // Custom build units are added in `build_unit_dependencies`.
        assert!(!target.is_custom_build());
        let target_mode = match initial_target_mode {
            CompileMode::Test => {
                if target.is_example() && !filter.is_specific() && !target.tested() {
                    // Examples are included as regular binaries to verify
                    // that they compile.
                    CompileMode::Build
                } else {
                    CompileMode::Test
                }
            }
            CompileMode::Build => match *target.kind() {
                TargetKind::Test => CompileMode::Test,
                TargetKind::Bench => CompileMode::Bench,
                _ => CompileMode::Build,
            },
            // `CompileMode::Bench` is only used to inform `filter_default_targets`
            // which command is being used (`cargo bench`). Afterwards, tests
            // and benches are treated identically. Switching the mode allows
            // de-duplication of units that are essentially identical. For
            // example, `cargo build --all-targets --release` creates the units
            // (lib profile:bench, mode:test) and (lib profile:bench, mode:bench)
            // and since these are the same, we want them to be de-duplicated in
            // `unit_dependencies`.
            CompileMode::Bench => CompileMode::Test,
            _ => initial_target_mode,
        };

        let is_local = pkg.package_id().source_id().is_path();

        // No need to worry about build-dependencies, roots are never build dependencies.
        let features_for = FeaturesFor::from_for_host(target.proc_macro());
        let features = resolved_features.activated_features(pkg.package_id(), features_for);

        // If `--target` has not been specified, then the unit
        // graph is built almost like if `--target $HOST` was
        // specified. See `rebuild_unit_graph_shared` for more on
        // why this is done. However, if the package has its own
        // `package.target` key, then this gets used instead of
        // `$HOST`
        let explicit_kinds = if let Some(k) = pkg.manifest().forced_kind() {
            vec![k]
        } else {
            requested_kinds
                .iter()
                .map(|kind| match kind {
                    CompileKind::Host => {
                        pkg.manifest().default_kind().unwrap_or(explicit_host_kind)
                    }
                    CompileKind::Target(t) => CompileKind::Target(*t),
                })
                .collect()
        };

        for kind in explicit_kinds.iter() {
            let unit_for = if initial_target_mode.is_any_test() {
                // NOTE: the `UnitFor` here is subtle. If you have a profile
                // with `panic` set, the `panic` flag is cleared for
                // tests/benchmarks and their dependencies. If this
                // was `normal`, then the lib would get compiled three
                // times (once with panic, once without, and once with
                // `--test`).
                //
                // This would cause a problem for doc tests, which would fail
                // because `rustdoc` would attempt to link with both libraries
                // at the same time. Also, it's probably not important (or
                // even desirable?) for rustdoc to link with a lib with
                // `panic` set.
                //
                // As a consequence, Examples and Binaries get compiled
                // without `panic` set. This probably isn't a bad deal.
                //
                // Forcing the lib to be compiled three times during `cargo
                // test` is probably also not desirable.
                UnitFor::new_test(config, *kind)
            } else if target.for_host() {
                // Proc macro / plugin should not have `panic` set.
                UnitFor::new_compiler(*kind)
            } else {
                UnitFor::new_normal(*kind)
            };
            let profile = profiles.get_profile(
                pkg.package_id(),
                ws.is_member(pkg),
                is_local,
                unit_for,
                *kind,
            );
            let unit = interner.intern(
                pkg,
                target,
                profile,
                kind.for_target(target),
                target_mode,
                features.clone(),
                /*is_std*/ false,
                /*dep_hash*/ 0,
                IsArtifact::No,
            );
            units.insert(unit);
        }
    };

    // Create a list of proposed targets.
    let mut proposals: Vec<Proposal<'_>> = Vec::new();

    match *filter {
        CompileFilter::Default {
            required_features_filterable,
        } => {
            for pkg in packages {
                let default = filter_default_targets(pkg.targets(), mode);
                proposals.extend(default.into_iter().map(|target| Proposal {
                    pkg,
                    target,
                    requires_features: !required_features_filterable,
                    mode,
                }));
                if mode == CompileMode::Test {
                    if let Some(t) = pkg
                        .targets()
                        .iter()
                        .find(|t| t.is_lib() && t.doctested() && t.doctestable())
                    {
                        proposals.push(Proposal {
                            pkg,
                            target: t,
                            requires_features: false,
                            mode: CompileMode::Doctest,
                        });
                    }
                }
            }
        }
        CompileFilter::Only {
            all_targets,
            ref lib,
            ref bins,
            ref examples,
            ref tests,
            ref benches,
        } => {
            if *lib != LibRule::False {
                let mut libs = Vec::new();
                for proposal in filter_targets(packages, Target::is_lib, false, mode) {
                    let Proposal { target, pkg, .. } = proposal;
                    if mode.is_doc_test() && !target.doctestable() {
                        let types = target.rustc_crate_types();
                        let types_str: Vec<&str> = types.iter().map(|t| t.as_str()).collect();
                        ws.config().shell().warn(format!(
                            "doc tests are not supported for crate type(s) `{}` in package `{}`",
                            types_str.join(", "),
                            pkg.name()
                        ))?;
                    } else {
                        libs.push(proposal)
                    }
                }
                if !all_targets && libs.is_empty() && *lib == LibRule::True {
                    let names = packages.iter().map(|pkg| pkg.name()).collect::<Vec<_>>();
                    if names.len() == 1 {
                        anyhow::bail!("no library targets found in package `{}`", names[0]);
                    } else {
                        anyhow::bail!("no library targets found in packages: {}", names.join(", "));
                    }
                }
                proposals.extend(libs);
            }

            // If `--tests` was specified, add all targets that would be
            // generated by `cargo test`.
            let test_filter = match tests {
                FilterRule::All => Target::tested,
                FilterRule::Just(_) => Target::is_test,
            };
            let test_mode = match mode {
                CompileMode::Build => CompileMode::Test,
                CompileMode::Check { .. } => CompileMode::Check { test: true },
                _ => mode,
            };
            // If `--benches` was specified, add all targets that would be
            // generated by `cargo bench`.
            let bench_filter = match benches {
                FilterRule::All => Target::benched,
                FilterRule::Just(_) => Target::is_bench,
            };
            let bench_mode = match mode {
                CompileMode::Build => CompileMode::Bench,
                CompileMode::Check { .. } => CompileMode::Check { test: true },
                _ => mode,
            };

            proposals.extend(list_rule_targets(
                packages,
                bins,
                "bin",
                Target::is_bin,
                mode,
            )?);
            proposals.extend(list_rule_targets(
                packages,
                examples,
                "example",
                Target::is_example,
                mode,
            )?);
            proposals.extend(list_rule_targets(
                packages,
                tests,
                "test",
                test_filter,
                test_mode,
            )?);
            proposals.extend(list_rule_targets(
                packages,
                benches,
                "bench",
                bench_filter,
                bench_mode,
            )?);
        }
    }

    // Only include targets that are libraries or have all required
    // features available.
    //
    // `features_map` is a map of &Package -> enabled_features
    // It is computed by the set of enabled features for the package plus
    // every enabled feature of every enabled dependency.
    let mut features_map = HashMap::new();
    // This needs to be a set to de-duplicate units. Due to the way the
    // targets are filtered, it is possible to have duplicate proposals for
    // the same thing.
    let mut units = HashSet::new();
    for Proposal {
        pkg,
        target,
        requires_features,
        mode,
    } in proposals
    {
        let unavailable_features = match target.required_features() {
            Some(rf) => {
                validate_required_features(
                    workspace_resolve,
                    target.name(),
                    rf,
                    pkg.summary(),
                    &mut config.shell(),
                )?;

                let features = features_map.entry(pkg).or_insert_with(|| {
                    resolve_all_features(resolve, resolved_features, package_set, pkg.package_id())
                });
                rf.iter().filter(|f| !features.contains(*f)).collect()
            }
            None => Vec::new(),
        };
        if target.is_lib() || unavailable_features.is_empty() {
            new_unit(&mut units, pkg, target, mode);
        } else if requires_features {
            let required_features = target.required_features().unwrap();
            let quoted_required_features: Vec<String> = required_features
                .iter()
                .map(|s| format!("`{}`", s))
                .collect();
            anyhow::bail!(
                "target `{}` in package `{}` requires the features: {}\n\
                 Consider enabling them by passing, e.g., `--features=\"{}\"`",
                target.name(),
                pkg.name(),
                quoted_required_features.join(", "),
                required_features.join(" ")
            );
        }
        // else, silently skip target.
    }
    let mut units: Vec<_> = units.into_iter().collect();
    unmatched_target_filters(&units, filter, &mut ws.config().shell())?;

    // Keep the roots in a consistent order, which helps with checking test output.
    units.sort_unstable();
    Ok(units)
}

/// Checks if the unit list is empty and the user has passed any combination of
/// --tests, --examples, --benches or --bins, and we didn't match on any targets.
/// We want to emit a warning to make sure the user knows that this run is a no-op,
/// and their code remains unchecked despite cargo not returning any errors
fn unmatched_target_filters(
    units: &[Unit],
    filter: &CompileFilter,
    shell: &mut Shell,
) -> CargoResult<()> {
    if let CompileFilter::Only {
        all_targets,
        lib: _,
        ref bins,
        ref examples,
        ref tests,
        ref benches,
    } = *filter
    {
        if units.is_empty() {
            let mut filters = String::new();
            let mut miss_count = 0;

            let mut append = |t: &FilterRule, s| {
                if let FilterRule::All = *t {
                    miss_count += 1;
                    filters.push_str(s);
                }
            };

            if all_targets {
                filters.push_str(" `all-targets`");
            } else {
                append(bins, " `bins`,");
                append(tests, " `tests`,");
                append(examples, " `examples`,");
                append(benches, " `benches`,");
                filters.pop();
            }

            return shell.warn(format!(
                "Target {}{} specified, but no targets matched. This is a no-op",
                if miss_count > 1 { "filters" } else { "filter" },
                filters,
            ));
        }
    }

    Ok(())
}

/// Warns if a target's required-features references a feature that doesn't exist.
///
/// This is a warning because historically this was not validated, and it
/// would cause too much breakage to make it an error.
fn validate_required_features(
    resolve: &Option<Resolve>,
    target_name: &str,
    required_features: &[String],
    summary: &Summary,
    shell: &mut Shell,
) -> CargoResult<()> {
    let resolve = match resolve {
        None => return Ok(()),
        Some(resolve) => resolve,
    };

    for feature in required_features {
        let fv = FeatureValue::new(feature.into());
        match &fv {
            FeatureValue::Feature(f) => {
                if !summary.features().contains_key(f) {
                    shell.warn(format!(
                        "invalid feature `{}` in required-features of target `{}`: \
                        `{}` is not present in [features] section",
                        fv, target_name, fv
                    ))?;
                }
            }
            FeatureValue::Dep { .. } => {
                anyhow::bail!(
                    "invalid feature `{}` in required-features of target `{}`: \
                    `dep:` prefixed feature values are not allowed in required-features",
                    fv,
                    target_name
                );
            }
            FeatureValue::DepFeature { weak: true, .. } => {
                anyhow::bail!(
                    "invalid feature `{}` in required-features of target `{}`: \
                    optional dependency with `?` is not allowed in required-features",
                    fv,
                    target_name
                );
            }
            // Handling of dependent_crate/dependent_crate_feature syntax
            FeatureValue::DepFeature {
                dep_name,
                dep_feature,
                weak: false,
            } => {
                match resolve
                    .deps(summary.package_id())
                    .find(|(_dep_id, deps)| deps.iter().any(|dep| dep.name_in_toml() == *dep_name))
                {
                    Some((dep_id, _deps)) => {
                        let dep_summary = resolve.summary(dep_id);
                        if !dep_summary.features().contains_key(dep_feature)
                            && !dep_summary
                                .dependencies()
                                .iter()
                                .any(|dep| dep.name_in_toml() == *dep_feature && dep.is_optional())
                        {
                            shell.warn(format!(
                                "invalid feature `{}` in required-features of target `{}`: \
                                feature `{}` does not exist in package `{}`",
                                fv, target_name, dep_feature, dep_id
                            ))?;
                        }
                    }
                    None => {
                        shell.warn(format!(
                            "invalid feature `{}` in required-features of target `{}`: \
                            dependency `{}` does not exist",
                            fv, target_name, dep_name
                        ))?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Gets all of the features enabled for a package, plus its dependencies'
/// features.
///
/// Dependencies are added as `dep_name/feat_name` because `required-features`
/// wants to support that syntax.
pub fn resolve_all_features(
    resolve_with_overrides: &Resolve,
    resolved_features: &features::ResolvedFeatures,
    package_set: &PackageSet<'_>,
    package_id: PackageId,
) -> HashSet<String> {
    let mut features: HashSet<String> = resolved_features
        .activated_features(package_id, FeaturesFor::NormalOrDev)
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Include features enabled for use by dependencies so targets can also use them with the
    // required-features field when deciding whether to be built or skipped.
    for (dep_id, deps) in resolve_with_overrides.deps(package_id) {
        let is_proc_macro = package_set
            .get_one(dep_id)
            .expect("packages downloaded")
            .proc_macro();
        for dep in deps {
            let features_for = FeaturesFor::from_for_host(is_proc_macro || dep.is_build());
            for feature in resolved_features
                .activated_features_unverified(dep_id, features_for)
                .unwrap_or_default()
            {
                features.insert(format!("{}/{}", dep.name_in_toml(), feature));
            }
        }
    }

    features
}

/// Given a list of all targets for a package, filters out only the targets
/// that are automatically included when the user doesn't specify any targets.
fn filter_default_targets(targets: &[Target], mode: CompileMode) -> Vec<&Target> {
    match mode {
        CompileMode::Bench => targets.iter().filter(|t| t.benched()).collect(),
        CompileMode::Test => targets
            .iter()
            .filter(|t| t.tested() || t.is_example())
            .collect(),
        CompileMode::Build | CompileMode::Check { .. } => targets
            .iter()
            .filter(|t| t.is_bin() || t.is_lib())
            .collect(),
        CompileMode::Doc { .. } => {
            // `doc` does lib and bins (bin with same name as lib is skipped).
            targets
                .iter()
                .filter(|t| {
                    t.documented()
                        && (!t.is_bin()
                            || !targets.iter().any(|l| l.is_lib() && l.name() == t.name()))
                })
                .collect()
        }
        CompileMode::Doctest | CompileMode::Docscrape | CompileMode::RunCustomBuild => {
            panic!("Invalid mode {:?}", mode)
        }
    }
}

/// Returns a list of proposed targets based on command-line target selection flags.
fn list_rule_targets<'a>(
    packages: &[&'a Package],
    rule: &FilterRule,
    target_desc: &'static str,
    is_expected_kind: fn(&Target) -> bool,
    mode: CompileMode,
) -> CargoResult<Vec<Proposal<'a>>> {
    let mut proposals = Vec::new();
    match rule {
        FilterRule::All => {
            proposals.extend(filter_targets(packages, is_expected_kind, false, mode))
        }
        FilterRule::Just(names) => {
            for name in names {
                proposals.extend(find_named_targets(
                    packages,
                    name,
                    target_desc,
                    is_expected_kind,
                    mode,
                )?);
            }
        }
    }
    Ok(proposals)
}

/// Finds the targets for a specifically named target.
fn find_named_targets<'a>(
    packages: &[&'a Package],
    target_name: &str,
    target_desc: &'static str,
    is_expected_kind: fn(&Target) -> bool,
    mode: CompileMode,
) -> CargoResult<Vec<Proposal<'a>>> {
    let is_glob = is_glob_pattern(target_name);
    let proposals = if is_glob {
        let pattern = build_glob(target_name)?;
        let filter = |t: &Target| is_expected_kind(t) && pattern.matches(t.name());
        filter_targets(packages, filter, true, mode)
    } else {
        let filter = |t: &Target| t.name() == target_name && is_expected_kind(t);
        filter_targets(packages, filter, true, mode)
    };

    if proposals.is_empty() {
        let targets = packages
            .iter()
            .flat_map(|pkg| {
                pkg.targets()
                    .iter()
                    .filter(|target| is_expected_kind(target))
            })
            .collect::<Vec<_>>();
        let suggestion = closest_msg(target_name, targets.iter(), |t| t.name());
        if !suggestion.is_empty() {
            anyhow::bail!(
                "no {} target {} `{}`{}",
                target_desc,
                if is_glob { "matches pattern" } else { "named" },
                target_name,
                suggestion
            );
        } else {
            let mut msg = String::new();
            writeln!(
                msg,
                "no {} target {} `{}`.",
                target_desc,
                if is_glob { "matches pattern" } else { "named" },
                target_name,
            )?;
            if !targets.is_empty() {
                writeln!(msg, "Available {} targets:", target_desc)?;
                for target in targets {
                    writeln!(msg, "    {}", target.name())?;
                }
            }
            anyhow::bail!(msg);
        }
    }
    Ok(proposals)
}

fn filter_targets<'a>(
    packages: &[&'a Package],
    predicate: impl Fn(&Target) -> bool,
    requires_features: bool,
    mode: CompileMode,
) -> Vec<Proposal<'a>> {
    let mut proposals = Vec::new();
    for pkg in packages {
        for target in pkg.targets().iter().filter(|t| predicate(t)) {
            proposals.push(Proposal {
                pkg,
                target,
                requires_features,
                mode,
            });
        }
    }
    proposals
}

/// This is used to rebuild the unit graph, sharing host dependencies if possible.
///
/// This will translate any unit's `CompileKind::Target(host)` to
/// `CompileKind::Host` if the kind is equal to `to_host`. This also handles
/// generating the unit `dep_hash`, and merging shared units if possible.
///
/// This is necessary because if normal dependencies used `CompileKind::Host`,
/// there would be no way to distinguish those units from build-dependency
/// units. This can cause a problem if a shared normal/build dependency needs
/// to link to another dependency whose features differ based on whether or
/// not it is a normal or build dependency. If both units used
/// `CompileKind::Host`, then they would end up being identical, causing a
/// collision in the `UnitGraph`, and Cargo would end up randomly choosing one
/// value or the other.
///
/// The solution is to keep normal and build dependencies separate when
/// building the unit graph, and then run this second pass which will try to
/// combine shared dependencies safely. By adding a hash of the dependencies
/// to the `Unit`, this allows the `CompileKind` to be changed back to `Host`
/// without fear of an unwanted collision.
fn rebuild_unit_graph_shared(
    interner: &UnitInterner,
    unit_graph: UnitGraph,
    roots: &[Unit],
    scrape_units: &[Unit],
    to_host: CompileKind,
) -> (Vec<Unit>, Vec<Unit>, UnitGraph) {
    let mut result = UnitGraph::new();
    // Map of the old unit to the new unit, used to avoid recursing into units
    // that have already been computed to improve performance.
    let mut memo = HashMap::new();
    let new_roots = roots
        .iter()
        .map(|root| {
            traverse_and_share(interner, &mut memo, &mut result, &unit_graph, root, to_host)
        })
        .collect();
    let new_scrape_units = scrape_units
        .iter()
        .map(|unit| memo.get(unit).unwrap().clone())
        .collect();
    (new_roots, new_scrape_units, result)
}

/// Recursive function for rebuilding the graph.
///
/// This walks `unit_graph`, starting at the given `unit`. It inserts the new
/// units into `new_graph`, and returns a new updated version of the given
/// unit (`dep_hash` is filled in, and `kind` switched if necessary).
fn traverse_and_share(
    interner: &UnitInterner,
    memo: &mut HashMap<Unit, Unit>,
    new_graph: &mut UnitGraph,
    unit_graph: &UnitGraph,
    unit: &Unit,
    to_host: CompileKind,
) -> Unit {
    if let Some(new_unit) = memo.get(unit) {
        // Already computed, no need to recompute.
        return new_unit.clone();
    }
    let mut dep_hash = StableHasher::new();
    let new_deps: Vec<_> = unit_graph[unit]
        .iter()
        .map(|dep| {
            let new_dep_unit =
                traverse_and_share(interner, memo, new_graph, unit_graph, &dep.unit, to_host);
            new_dep_unit.hash(&mut dep_hash);
            UnitDep {
                unit: new_dep_unit,
                ..dep.clone()
            }
        })
        .collect();
    let new_dep_hash = dep_hash.finish();
    let new_kind = if unit.kind == to_host {
        CompileKind::Host
    } else {
        unit.kind
    };
    let new_unit = interner.intern(
        &unit.pkg,
        &unit.target,
        unit.profile.clone(),
        new_kind,
        unit.mode,
        unit.features.clone(),
        unit.is_std,
        new_dep_hash,
        unit.artifact,
    );
    assert!(memo.insert(unit.clone(), new_unit.clone()).is_none());
    new_graph.entry(new_unit.clone()).or_insert(new_deps);
    new_unit
}

/// Removes duplicate CompileMode::Doc units that would cause problems with
/// filename collisions.
///
/// Rustdoc only separates units by crate name in the file directory
/// structure. If any two units with the same crate name exist, this would
/// cause a filename collision, causing different rustdoc invocations to stomp
/// on one another's files.
///
/// Unfortunately this does not remove all duplicates, as some of them are
/// either user error, or difficult to remove. Cases that I can think of:
///
/// - Same target name in different packages. See the `collision_doc` test.
/// - Different sources. See `collision_doc_sources` test.
///
/// Ideally this would not be necessary.
fn remove_duplicate_doc(
    build_config: &BuildConfig,
    root_units: &[Unit],
    unit_graph: &mut UnitGraph,
) {
    // First, create a mapping of crate_name -> Unit so we can see where the
    // duplicates are.
    let mut all_docs: HashMap<String, Vec<Unit>> = HashMap::new();
    for unit in unit_graph.keys() {
        if unit.mode.is_doc() {
            all_docs
                .entry(unit.target.crate_name())
                .or_default()
                .push(unit.clone());
        }
    }
    // Keep track of units to remove so that they can be efficiently removed
    // from the unit_deps.
    let mut removed_units: HashSet<Unit> = HashSet::new();
    let mut remove = |units: Vec<Unit>, reason: &str, cb: &dyn Fn(&Unit) -> bool| -> Vec<Unit> {
        let (to_remove, remaining_units): (Vec<Unit>, Vec<Unit>) = units
            .into_iter()
            .partition(|unit| cb(unit) && !root_units.contains(unit));
        for unit in to_remove {
            log::debug!(
                "removing duplicate doc due to {} for package {} target `{}`",
                reason,
                unit.pkg,
                unit.target.name()
            );
            unit_graph.remove(&unit);
            removed_units.insert(unit);
        }
        remaining_units
    };
    // Iterate over the duplicates and try to remove them from unit_graph.
    for (_crate_name, mut units) in all_docs {
        if units.len() == 1 {
            continue;
        }
        // Prefer target over host if --target was not specified.
        if build_config
            .requested_kinds
            .iter()
            .all(CompileKind::is_host)
        {
            // Note these duplicates may not be real duplicates, since they
            // might get merged in rebuild_unit_graph_shared. Either way, it
            // shouldn't hurt to remove them early (although the report in the
            // log might be confusing).
            units = remove(units, "host/target merger", &|unit| unit.kind.is_host());
            if units.len() == 1 {
                continue;
            }
        }
        // Prefer newer versions over older.
        let mut source_map: HashMap<(InternedString, SourceId, CompileKind), Vec<Unit>> =
            HashMap::new();
        for unit in units {
            let pkg_id = unit.pkg.package_id();
            // Note, this does not detect duplicates from different sources.
            source_map
                .entry((pkg_id.name(), pkg_id.source_id(), unit.kind))
                .or_default()
                .push(unit);
        }
        let mut remaining_units = Vec::new();
        for (_key, mut units) in source_map {
            if units.len() > 1 {
                units.sort_by(|a, b| a.pkg.version().partial_cmp(b.pkg.version()).unwrap());
                // Remove any entries with version < newest.
                let newest_version = units.last().unwrap().pkg.version().clone();
                let keep_units = remove(units, "older version", &|unit| {
                    unit.pkg.version() < &newest_version
                });
                remaining_units.extend(keep_units);
            } else {
                remaining_units.extend(units);
            }
        }
        if remaining_units.len() == 1 {
            continue;
        }
        // Are there other heuristics to remove duplicates that would make
        // sense? Maybe prefer path sources over all others?
    }
    // Also remove units from the unit_deps so there aren't any dangling edges.
    for unit_deps in unit_graph.values_mut() {
        unit_deps.retain(|unit_dep| !removed_units.contains(&unit_dep.unit));
    }
    // Remove any orphan units that were detached from the graph.
    let mut visited = HashSet::new();
    fn visit(unit: &Unit, graph: &UnitGraph, visited: &mut HashSet<Unit>) {
        if !visited.insert(unit.clone()) {
            return;
        }
        for dep in &graph[unit] {
            visit(&dep.unit, graph, visited);
        }
    }
    for unit in root_units {
        visit(unit, unit_graph, &mut visited);
    }
    unit_graph.retain(|unit, _| visited.contains(unit));
}

/// Override crate types for given units.
///
/// This is primarily used by `cargo rustc --crate-type`.
fn override_rustc_crate_types(
    units: &mut [Unit],
    args: &[String],
    interner: &UnitInterner,
) -> CargoResult<()> {
    if units.len() != 1 {
        anyhow::bail!(
            "crate types to rustc can only be passed to one \
            target, consider filtering\nthe package by passing, \
            e.g., `--lib` or `--example` to specify a single target"
        );
    }

    let unit = &units[0];
    let override_unit = |f: fn(Vec<CrateType>) -> TargetKind| {
        let crate_types = args.iter().map(|s| s.into()).collect();
        let mut target = unit.target.clone();
        target.set_kind(f(crate_types));
        interner.intern(
            &unit.pkg,
            &target,
            unit.profile.clone(),
            unit.kind,
            unit.mode,
            unit.features.clone(),
            unit.is_std,
            unit.dep_hash,
            unit.artifact,
        )
    };
    units[0] = match unit.target.kind() {
        TargetKind::Lib(_) => override_unit(TargetKind::Lib),
        TargetKind::ExampleLib(_) => override_unit(TargetKind::ExampleLib),
        _ => {
            anyhow::bail!(
                "crate types can only be specified for libraries and example libraries.\n\
                Binaries, tests, and benchmarks are always the `bin` crate type"
            );
        }
    };

    Ok(())
}
