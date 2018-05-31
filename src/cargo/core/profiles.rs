use std::collections::{HashMap, HashSet};
use std::sync::atomic;
use std::{cmp, fmt, hash};

use core::compiler::CompileMode;
use core::interning::InternedString;
use core::{Features, PackageId, PackageIdSpec, PackageSet, Shell};
use util::lev_distance::lev_distance;
use util::toml::{ProfilePackageSpec, StringOrBool, TomlProfile, TomlProfiles, U32OrBool};
use util::{CargoResult, Config, ConfigValue};

/// Collection of all user profiles.
#[derive(Clone, Debug)]
pub struct Profiles {
    dev: ProfileMaker,
    release: ProfileMaker,
    test: ProfileMaker,
    bench: ProfileMaker,
    doc: ProfileMaker,
}

impl Profiles {
    pub fn new(
        profiles: Option<&TomlProfiles>,
        config: &Config,
        features: &Features,
        warnings: &mut Vec<String>,
    ) -> CargoResult<Profiles> {
        if let Some(profiles) = profiles {
            profiles.validate(features, warnings)?;
        }
        Profiles::validate_config(config, warnings)?;

        Ok(Profiles {
            dev: ProfileMaker {
                default: Profile::default_dev(),
                toml: profiles.and_then(|p| p.dev.clone()),
                config: TomlProfile::from_config(config, "dev", warnings)?,
            },
            release: ProfileMaker {
                default: Profile::default_release(),
                toml: profiles.and_then(|p| p.release.clone()),
                config: TomlProfile::from_config(config, "release", warnings)?,
            },
            test: ProfileMaker {
                default: Profile::default_test(),
                toml: profiles.and_then(|p| p.test.clone()),
                config: None,
            },
            bench: ProfileMaker {
                default: Profile::default_bench(),
                toml: profiles.and_then(|p| p.bench.clone()),
                config: None,
            },
            doc: ProfileMaker {
                default: Profile::default_doc(),
                toml: profiles.and_then(|p| p.doc.clone()),
                config: None,
            },
        })
    }

    /// Retrieve the profile for a target.
    /// `is_member` is whether or not this package is a member of the
    /// workspace.
    pub fn get_profile(
        &self,
        pkg_id: &PackageId,
        is_member: bool,
        profile_for: ProfileFor,
        mode: CompileMode,
        release: bool,
    ) -> Profile {
        let maker = match mode {
            CompileMode::Test => {
                if release {
                    &self.bench
                } else {
                    &self.test
                }
            }
            CompileMode::Build
            | CompileMode::Check { .. }
            | CompileMode::Doctest
            | CompileMode::RunCustomBuild => {
                // Note: RunCustomBuild doesn't normally use this code path.
                // `build_unit_profiles` normally ensures that it selects the
                // ancestor's profile.  However `cargo clean -p` can hit this
                // path.
                if release {
                    &self.release
                } else {
                    &self.dev
                }
            }
            CompileMode::Bench => &self.bench,
            CompileMode::Doc { .. } => &self.doc,
        };
        let mut profile = maker.get_profile(Some(pkg_id), is_member, profile_for);
        // `panic` should not be set for tests/benches, or any of their
        // dependencies.
        if profile_for == ProfileFor::TestDependency || mode.is_any_test() {
            profile.panic = None;
        }
        profile
    }

    /// The profile for *running* a `build.rs` script is only used for setting
    /// a few environment variables.  To ensure proper de-duplication of the
    /// running `Unit`, this uses a stripped-down profile (so that unrelated
    /// profile flags don't cause `build.rs` to needlessly run multiple
    /// times).
    pub fn get_profile_run_custom_build(&self, for_unit_profile: &Profile) -> Profile {
        let mut result = Profile::default();
        result.debuginfo = for_unit_profile.debuginfo;
        result.opt_level = for_unit_profile.opt_level;
        result
    }

    /// This returns a generic base profile. This is currently used for the
    /// `[Finished]` line.  It is not entirely accurate, since it doesn't
    /// select for the package that was actually built.
    pub fn base_profile(&self, release: bool) -> Profile {
        if release {
            self.release.get_profile(None, true, ProfileFor::Any)
        } else {
            self.dev.get_profile(None, true, ProfileFor::Any)
        }
    }

    /// Used to check for overrides for non-existing packages.
    pub fn validate_packages(&self, shell: &mut Shell, packages: &PackageSet) -> CargoResult<()> {
        self.dev.validate_packages(shell, packages)?;
        self.release.validate_packages(shell, packages)?;
        self.test.validate_packages(shell, packages)?;
        self.bench.validate_packages(shell, packages)?;
        self.doc.validate_packages(shell, packages)?;
        Ok(())
    }

    fn validate_config(config: &Config, warnings: &mut Vec<String>) -> CargoResult<()> {
        static VALIDATE_ONCE: atomic::AtomicBool = atomic::ATOMIC_BOOL_INIT;

        if VALIDATE_ONCE.swap(true, atomic::Ordering::SeqCst) {
            return Ok(());
        }

        // cv: Value<HashMap<String, CV>>
        if let Some(cv) = config.get_table("profile")? {
            // Warn if config profiles without CLI option.
            if !config.cli_unstable().config_profile {
                warnings.push(format!(
                    "profile in config `{}` requires `-Z config-profile` command-line option",
                    cv.definition
                ));
                // Ignore the rest.
                return Ok(());
            }
            // Warn about unsupported profile names.
            for (key, profile_cv) in cv.val.iter() {
                if key != "dev" && key != "release" {
                    warnings.push(format!(
                        "profile `{}` in config `{}` is not supported",
                        key,
                        profile_cv.definition_path().display()
                    ));
                }
            }
            // Warn about incorrect key names.
            for profile_cv in cv.val.values() {
                if let ConfigValue::Table(ref profile, _) = *profile_cv {
                    validate_profile_keys(profile, warnings);
                    if let Some(&ConfigValue::Table(ref bo_profile, _)) =
                        profile.get("build-override")
                    {
                        validate_profile_keys(bo_profile, warnings);
                    }
                    if let Some(&ConfigValue::Table(ref os, _)) = profile.get("overrides") {
                        for o_profile_cv in os.values() {
                            if let ConfigValue::Table(ref o_profile, _) = *o_profile_cv {
                                validate_profile_keys(o_profile, warnings);
                            }
                        }
                    }
                }
            }
        }
        return Ok(());

        fn validate_profile_keys(
            profile: &HashMap<String, ConfigValue>,
            warnings: &mut Vec<String>,
        ) {
            for (key, value_cv) in profile.iter() {
                if !TOML_PROFILE_KEYS.iter().any(|k| k == key) {
                    warnings.push(format!(
                        "unused profile key `{}` in config `{}`",
                        key,
                        value_cv.definition_path().display()
                    ));
                }
            }
        }
    }
}

/// An object used for handling the profile override hierarchy.
///
/// The precedence of profiles are (first one wins):
/// - Profiles in .cargo/config files (using same order as below).
/// - [profile.dev.overrides.name] - A named package.
/// - [profile.dev.overrides."*"] - This cannot apply to workspace members.
/// - [profile.dev.build-override] - This can only apply to `build.rs` scripts
///   and their dependencies.
/// - [profile.dev]
/// - Default (hard-coded) values.
#[derive(Debug, Clone)]
struct ProfileMaker {
    /// The starting, hard-coded defaults for the profile.
    default: Profile,
    /// The profile from the `Cargo.toml` manifest.
    toml: Option<TomlProfile>,
    /// Profile loaded from `.cargo/config` files.
    config: Option<TomlProfile>,
}

impl ProfileMaker {
    fn get_profile(
        &self,
        pkg_id: Option<&PackageId>,
        is_member: bool,
        profile_for: ProfileFor,
    ) -> Profile {
        let mut profile = self.default;
        if let Some(ref toml) = self.toml {
            merge_toml(pkg_id, is_member, profile_for, &mut profile, toml);
        }
        if let Some(ref toml) = self.config {
            merge_toml(pkg_id, is_member, profile_for, &mut profile, toml);
        }
        profile
    }

    fn validate_packages(&self, shell: &mut Shell, packages: &PackageSet) -> CargoResult<()> {
        self.validate_packages_toml(shell, packages, &self.toml, true)?;
        self.validate_packages_toml(shell, packages, &self.config, false)?;
        Ok(())
    }

    fn validate_packages_toml(
        &self,
        shell: &mut Shell,
        packages: &PackageSet,
        toml: &Option<TomlProfile>,
        warn_unmatched: bool,
    ) -> CargoResult<()> {
        let toml = match *toml {
            Some(ref toml) => toml,
            None => return Ok(()),
        };
        let overrides = match toml.overrides {
            Some(ref overrides) => overrides,
            None => return Ok(()),
        };
        // Verify that a package doesn't match multiple spec overrides.
        let mut found = HashSet::new();
        for pkg_id in packages.package_ids() {
            let matches: Vec<&PackageIdSpec> = overrides
                .keys()
                .filter_map(|key| match *key {
                    ProfilePackageSpec::All => None,
                    ProfilePackageSpec::Spec(ref spec) => if spec.matches(pkg_id) {
                        Some(spec)
                    } else {
                        None
                    },
                })
                .collect();
            match matches.len() {
                0 => {}
                1 => {
                    found.insert(matches[0].clone());
                }
                _ => {
                    let specs = matches
                        .iter()
                        .map(|spec| spec.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    bail!(
                        "multiple profile overrides in profile `{}` match package `{}`\n\
                         found profile override specs: {}",
                        self.default.name,
                        pkg_id,
                        specs
                    );
                }
            }
        }

        if !warn_unmatched {
            return Ok(());
        }
        // Verify every override matches at least one package.
        let missing_specs = overrides.keys().filter_map(|key| {
            if let ProfilePackageSpec::Spec(ref spec) = *key {
                if !found.contains(spec) {
                    return Some(spec);
                }
            }
            None
        });
        for spec in missing_specs {
            // See if there is an exact name match.
            let name_matches: Vec<String> = packages
                .package_ids()
                .filter_map(|pkg_id| {
                    if pkg_id.name().as_str() == spec.name() {
                        Some(pkg_id.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            if name_matches.is_empty() {
                let suggestion = packages
                    .package_ids()
                    .map(|p| (lev_distance(spec.name(), &p.name()), p.name()))
                    .filter(|&(d, _)| d < 4)
                    .min_by_key(|p| p.0)
                    .map(|p| p.1);
                match suggestion {
                    Some(p) => shell.warn(format!(
                        "profile override spec `{}` did not match any packages\n\n\
                         Did you mean `{}`?",
                        spec, p
                    ))?,
                    None => shell.warn(format!(
                        "profile override spec `{}` did not match any packages",
                        spec
                    ))?,
                }
            } else {
                shell.warn(format!(
                    "version or URL in profile override spec `{}` does not \
                     match any of the packages: {}",
                    spec,
                    name_matches.join(", ")
                ))?;
            }
        }
        Ok(())
    }
}

fn merge_toml(
    pkg_id: Option<&PackageId>,
    is_member: bool,
    profile_for: ProfileFor,
    profile: &mut Profile,
    toml: &TomlProfile,
) {
    merge_profile(profile, toml);
    if profile_for == ProfileFor::CustomBuild {
        if let Some(ref build_override) = toml.build_override {
            merge_profile(profile, build_override);
        }
    }
    if let Some(ref overrides) = toml.overrides {
        if !is_member {
            if let Some(all) = overrides.get(&ProfilePackageSpec::All) {
                merge_profile(profile, all);
            }
        }
        if let Some(pkg_id) = pkg_id {
            let mut matches = overrides
                .iter()
                .filter_map(|(key, spec_profile)| match *key {
                    ProfilePackageSpec::All => None,
                    ProfilePackageSpec::Spec(ref s) => if s.matches(pkg_id) {
                        Some(spec_profile)
                    } else {
                        None
                    },
                });
            if let Some(spec_profile) = matches.next() {
                merge_profile(profile, spec_profile);
                // `validate_packages` should ensure that there are
                // no additional matches.
                assert!(
                    matches.next().is_none(),
                    "package `{}` matched multiple profile overrides",
                    pkg_id
                );
            }
        }
    }
}

fn merge_profile(profile: &mut Profile, toml: &TomlProfile) {
    if let Some(ref opt_level) = toml.opt_level {
        profile.opt_level = InternedString::new(&opt_level.0);
    }
    match toml.lto {
        Some(StringOrBool::Bool(b)) => profile.lto = Lto::Bool(b),
        Some(StringOrBool::String(ref n)) => profile.lto = Lto::Named(InternedString::new(n)),
        None => {}
    }
    if toml.codegen_units.is_some() {
        profile.codegen_units = toml.codegen_units;
    }
    match toml.debug {
        Some(U32OrBool::U32(debug)) => profile.debuginfo = Some(debug),
        Some(U32OrBool::Bool(true)) => profile.debuginfo = Some(2),
        Some(U32OrBool::Bool(false)) => profile.debuginfo = None,
        None => {}
    }
    if let Some(debug_assertions) = toml.debug_assertions {
        profile.debug_assertions = debug_assertions;
    }
    if let Some(rpath) = toml.rpath {
        profile.rpath = rpath;
    }
    if let Some(ref panic) = toml.panic {
        profile.panic = Some(InternedString::new(panic));
    }
    if let Some(overflow_checks) = toml.overflow_checks {
        profile.overflow_checks = overflow_checks;
    }
    if let Some(incremental) = toml.incremental {
        profile.incremental = incremental;
    }
}

/// Profile settings used to determine which compiler flags to use for a
/// target.
#[derive(Debug, Clone, Copy, Eq)]
pub struct Profile {
    pub name: &'static str,
    pub opt_level: InternedString,
    pub lto: Lto,
    // None = use rustc default
    pub codegen_units: Option<u32>,
    pub debuginfo: Option<u32>,
    pub debug_assertions: bool,
    pub overflow_checks: bool,
    pub rpath: bool,
    pub incremental: bool,
    pub panic: Option<InternedString>,
}

const TOML_PROFILE_KEYS: [&str; 11] = [
    "opt-level",
    "lto",
    "codegen-units",
    "debug",
    "debug-assertions",
    "rpath",
    "panic",
    "overflow-checks",
    "incremental",
    "overrides",
    "build-override",
];

impl Default for Profile {
    fn default() -> Profile {
        Profile {
            name: "",
            opt_level: InternedString::new("0"),
            lto: Lto::Bool(false),
            codegen_units: None,
            debuginfo: None,
            debug_assertions: false,
            overflow_checks: false,
            rpath: false,
            incremental: false,
            panic: None,
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Profile({})", self.name)
    }
}

impl hash::Hash for Profile {
    fn hash<H>(&self, state: &mut H)
    where
        H: hash::Hasher,
    {
        self.comparable().hash(state);
    }
}

impl cmp::PartialEq for Profile {
    fn eq(&self, other: &Self) -> bool {
        self.comparable() == other.comparable()
    }
}

impl Profile {
    fn default_dev() -> Profile {
        Profile {
            name: "dev",
            debuginfo: Some(2),
            debug_assertions: true,
            overflow_checks: true,
            incremental: true,
            ..Profile::default()
        }
    }

    fn default_release() -> Profile {
        Profile {
            name: "release",
            opt_level: InternedString::new("3"),
            ..Profile::default()
        }
    }

    fn default_test() -> Profile {
        Profile {
            name: "test",
            ..Profile::default_dev()
        }
    }

    fn default_bench() -> Profile {
        Profile {
            name: "bench",
            ..Profile::default_release()
        }
    }

    fn default_doc() -> Profile {
        Profile {
            name: "doc",
            ..Profile::default_dev()
        }
    }

    /// Compare all fields except `name`, which doesn't affect compilation.
    /// This is necessary for `Unit` deduplication for things like "test" and
    /// "dev" which are essentially the same.
    fn comparable(
        &self,
    ) -> (
        &InternedString,
        &Lto,
        &Option<u32>,
        &Option<u32>,
        &bool,
        &bool,
        &bool,
        &bool,
        &Option<InternedString>,
    ) {
        (
            &self.opt_level,
            &self.lto,
            &self.codegen_units,
            &self.debuginfo,
            &self.debug_assertions,
            &self.overflow_checks,
            &self.rpath,
            &self.incremental,
            &self.panic,
        )
    }
}

/// The link-time-optimization setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Lto {
    /// False = no LTO
    /// True = "Fat" LTO
    Bool(bool),
    /// Named LTO settings like "thin".
    Named(InternedString),
}

/// A flag used in `Unit` to indicate the purpose for the target.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ProfileFor {
    /// A general-purpose target.
    Any,
    /// A target for `build.rs` or any of its dependencies.  This enables
    /// `build-override` profiles for these targets.
    CustomBuild,
    /// A target that is a dependency of a test or benchmark.  Currently this
    /// enforces that the `panic` setting is not set.
    TestDependency,
}

impl ProfileFor {
    pub fn all_values() -> &'static [ProfileFor] {
        static ALL: [ProfileFor; 3] = [
            ProfileFor::Any,
            ProfileFor::CustomBuild,
            ProfileFor::TestDependency,
        ];
        &ALL
    }
}
