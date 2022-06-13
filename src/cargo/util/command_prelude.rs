use crate::core::compiler::{BuildConfig, MessageFormat, TimingOutput};
use crate::core::resolver::CliFeatures;
use crate::core::{Edition, Workspace};
use crate::ops::{CompileFilter, CompileOptions, NewOptions, Packages, VersionControl};
use crate::sources::CRATES_IO_REGISTRY;
use crate::util::important_paths::find_root_manifest_for_wd;
use crate::util::interning::InternedString;
use crate::util::restricted_names::is_glob_pattern;
use crate::util::toml::{StringOrVec, TomlProfile};
use crate::util::validate_package_name;
use crate::util::{
    print_available_benches, print_available_binaries, print_available_examples,
    print_available_packages, print_available_tests,
};
use crate::CargoResult;
use anyhow::bail;
use cargo_util::paths;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub use crate::core::compiler::CompileMode;
pub use crate::{CliError, CliResult, Config};
pub use clap::{AppSettings, Arg, ArgMatches};

pub type App = clap::Command<'static>;

pub trait AppExt: Sized {
    fn _arg(self, arg: Arg<'static>) -> Self;

    /// Do not use this method, it is only for backwards compatibility.
    /// Use `arg_package_spec_no_all` instead.
    fn arg_package_spec(
        self,
        package: &'static str,
        all: &'static str,
        exclude: &'static str,
    ) -> Self {
        self.arg_package_spec_no_all(package, all, exclude)
            ._arg(opt("all", "Alias for --workspace (deprecated)"))
    }

    /// Variant of arg_package_spec that does not include the `--all` flag
    /// (but does include `--workspace`). Used to avoid confusion with
    /// historical uses of `--all`.
    fn arg_package_spec_no_all(
        self,
        package: &'static str,
        all: &'static str,
        exclude: &'static str,
    ) -> Self {
        self.arg_package_spec_simple(package)
            ._arg(opt("workspace", all))
            ._arg(multi_opt("exclude", "SPEC", exclude))
    }

    fn arg_package_spec_simple(self, package: &'static str) -> Self {
        self._arg(optional_multi_opt("package", "SPEC", package).short('p'))
    }

    fn arg_package(self, package: &'static str) -> Self {
        self._arg(
            optional_opt("package", package)
                .short('p')
                .value_name("SPEC"),
        )
    }

    fn arg_jobs(self) -> Self {
        self._arg(
            opt("jobs", "Number of parallel jobs, defaults to # of CPUs")
                .short('j')
                .value_name("N"),
        )
        ._arg(opt(
            "keep-going",
            "Do not abort the build as soon as there is an error (unstable)",
        ))
    }

    fn arg_targets_all(
        self,
        lib: &'static str,
        bin: &'static str,
        bins: &'static str,
        example: &'static str,
        examples: &'static str,
        test: &'static str,
        tests: &'static str,
        bench: &'static str,
        benches: &'static str,
        all: &'static str,
    ) -> Self {
        self.arg_targets_lib_bin_example(lib, bin, bins, example, examples)
            ._arg(optional_multi_opt("test", "NAME", test))
            ._arg(opt("tests", tests))
            ._arg(optional_multi_opt("bench", "NAME", bench))
            ._arg(opt("benches", benches))
            ._arg(opt("all-targets", all))
    }

    fn arg_targets_lib_bin_example(
        self,
        lib: &'static str,
        bin: &'static str,
        bins: &'static str,
        example: &'static str,
        examples: &'static str,
    ) -> Self {
        self._arg(opt("lib", lib))
            ._arg(optional_multi_opt("bin", "NAME", bin))
            ._arg(opt("bins", bins))
            ._arg(optional_multi_opt("example", "NAME", example))
            ._arg(opt("examples", examples))
    }

    fn arg_targets_bins_examples(
        self,
        bin: &'static str,
        bins: &'static str,
        example: &'static str,
        examples: &'static str,
    ) -> Self {
        self._arg(optional_multi_opt("bin", "NAME", bin))
            ._arg(opt("bins", bins))
            ._arg(optional_multi_opt("example", "NAME", example))
            ._arg(opt("examples", examples))
    }

    fn arg_targets_bin_example(self, bin: &'static str, example: &'static str) -> Self {
        self._arg(optional_multi_opt("bin", "NAME", bin))
            ._arg(optional_multi_opt("example", "NAME", example))
    }

    fn arg_features(self) -> Self {
        self._arg(
            multi_opt(
                "features",
                "FEATURES",
                "Space or comma separated list of features to activate",
            )
            .short('F'),
        )
        ._arg(opt("all-features", "Activate all available features"))
        ._arg(opt(
            "no-default-features",
            "Do not activate the `default` feature",
        ))
    }

    fn arg_release(self, release: &'static str) -> Self {
        self._arg(opt("release", release).short('r'))
    }

    fn arg_profile(self, profile: &'static str) -> Self {
        self._arg(opt("profile", profile).value_name("PROFILE-NAME"))
    }

    fn arg_doc(self, doc: &'static str) -> Self {
        self._arg(opt("doc", doc))
    }

    fn arg_target_triple(self, target: &'static str) -> Self {
        self._arg(multi_opt("target", "TRIPLE", target))
    }

    fn arg_target_dir(self) -> Self {
        self._arg(
            opt("target-dir", "Directory for all generated artifacts").value_name("DIRECTORY"),
        )
    }

    fn arg_manifest_path(self) -> Self {
        self._arg(opt("manifest-path", "Path to Cargo.toml").value_name("PATH"))
    }

    fn arg_message_format(self) -> Self {
        self._arg(multi_opt("message-format", "FMT", "Error format"))
    }

    fn arg_build_plan(self) -> Self {
        self._arg(opt(
            "build-plan",
            "Output the build plan in JSON (unstable)",
        ))
    }

    fn arg_unit_graph(self) -> Self {
        self._arg(opt("unit-graph", "Output build graph in JSON (unstable)"))
    }

    fn arg_new_opts(self) -> Self {
        self._arg(
            opt(
                "vcs",
                "Initialize a new repository for the given version \
                 control system (git, hg, pijul, or fossil) or do not \
                 initialize any version control at all (none), overriding \
                 a global configuration.",
            )
            .value_name("VCS")
            .possible_values(&["git", "hg", "pijul", "fossil", "none"]),
        )
        ._arg(opt("bin", "Use a binary (application) template [default]"))
        ._arg(opt("lib", "Use a library template"))
        ._arg(
            opt("edition", "Edition to set for the crate generated")
                .possible_values(Edition::CLI_VALUES)
                .value_name("YEAR"),
        )
        ._arg(
            opt(
                "name",
                "Set the resulting package name, defaults to the directory name",
            )
            .value_name("NAME"),
        )
    }

    fn arg_index(self) -> Self {
        self._arg(opt("index", "Registry index URL to upload the package to").value_name("INDEX"))
    }

    fn arg_dry_run(self, dry_run: &'static str) -> Self {
        self._arg(opt("dry-run", dry_run))
    }

    fn arg_ignore_rust_version(self) -> Self {
        self._arg(opt(
            "ignore-rust-version",
            "Ignore `rust-version` specification in packages",
        ))
    }

    fn arg_future_incompat_report(self) -> Self {
        self._arg(opt(
            "future-incompat-report",
            "Outputs a future incompatibility report at the end of the build",
        ))
    }

    fn arg_quiet(self) -> Self {
        self._arg(opt("quiet", "Do not print cargo log messages").short('q'))
    }

    fn arg_timings(self) -> Self {
        self._arg(
            optional_opt(
                "timings",
                "Timing output formats (unstable) (comma separated): html, json",
            )
            .value_name("FMTS")
            .require_equals(true),
        )
    }
}

impl AppExt for App {
    fn _arg(self, arg: Arg<'static>) -> Self {
        self.arg(arg)
    }
}

pub fn opt(name: &'static str, help: &'static str) -> Arg<'static> {
    Arg::new(name).long(name).help(help)
}

pub fn optional_opt(name: &'static str, help: &'static str) -> Arg<'static> {
    opt(name, help).min_values(0)
}

pub fn optional_multi_opt(
    name: &'static str,
    value_name: &'static str,
    help: &'static str,
) -> Arg<'static> {
    opt(name, help)
        .value_name(value_name)
        .multiple_occurrences(true)
        .multiple_values(true)
        .min_values(0)
        .number_of_values(1)
}

pub fn multi_opt(name: &'static str, value_name: &'static str, help: &'static str) -> Arg<'static> {
    opt(name, help)
        .value_name(value_name)
        .multiple_occurrences(true)
}

pub fn subcommand(name: &'static str) -> App {
    App::new(name)
        .dont_collapse_args_in_usage(true)
        .setting(AppSettings::DeriveDisplayOrder)
}

/// Determines whether or not to gate `--profile` as unstable when resolving it.
pub enum ProfileChecking {
    /// `cargo rustc` historically has allowed "test", "bench", and "check". This
    /// variant explicitly allows those.
    LegacyRustc,
    /// `cargo check` and `cargo fix` historically has allowed "test". This variant
    /// explicitly allows that on stable.
    LegacyTestOnly,
    /// All other commands, which allow any valid custom named profile.
    Custom,
}

pub trait ArgMatchesExt {
    fn value_of_u32(&self, name: &str) -> CargoResult<Option<u32>> {
        let arg = match self._value_of(name) {
            None => None,
            Some(arg) => Some(arg.parse::<u32>().map_err(|_| {
                clap::Error::raw(
                    clap::ErrorKind::ValueValidation,
                    format!("Invalid value: could not parse `{}` as a number", arg),
                )
            })?),
        };
        Ok(arg)
    }

    /// Returns value of the `name` command-line argument as an absolute path
    fn value_of_path(&self, name: &str, config: &Config) -> Option<PathBuf> {
        self._value_of(name).map(|path| config.cwd().join(path))
    }

    fn root_manifest(&self, config: &Config) -> CargoResult<PathBuf> {
        if let Some(path) = self
            ._is_valid_arg("manifest-path")
            .then(|| self.value_of_path("manifest-path", config))
            .flatten()
        {
            // In general, we try to avoid normalizing paths in Cargo,
            // but in this particular case we need it to fix #3586.
            let path = paths::normalize_path(&path);
            if !path.ends_with("Cargo.toml") {
                anyhow::bail!("the manifest-path must be a path to a Cargo.toml file")
            }
            if !path.exists() {
                anyhow::bail!(
                    "manifest path `{}` does not exist",
                    self._value_of("manifest-path").unwrap()
                )
            }
            return Ok(path);
        }
        find_root_manifest_for_wd(config.cwd())
    }

    fn workspace<'a>(&self, config: &'a Config) -> CargoResult<Workspace<'a>> {
        let root = self.root_manifest(config)?;
        let mut ws = Workspace::new(&root, config)?;
        if config.cli_unstable().avoid_dev_deps {
            ws.set_require_optional_deps(false);
        }
        Ok(ws)
    }

    fn jobs(&self) -> CargoResult<Option<u32>> {
        self.value_of_u32("jobs")
    }

    fn verbose(&self) -> u32 {
        self._occurrences("verbose")
    }

    fn keep_going(&self) -> bool {
        self._is_present("keep-going")
    }

    fn targets(&self) -> Vec<String> {
        self._values_of("target")
    }

    fn get_profile_name(
        &self,
        config: &Config,
        default: &str,
        profile_checking: ProfileChecking,
    ) -> CargoResult<InternedString> {
        let specified_profile = self._value_of("profile");

        // Check for allowed legacy names.
        // This is an early exit, since it allows combination with `--release`.
        match (specified_profile, profile_checking) {
            // `cargo rustc` has legacy handling of these names
            (Some(name @ ("dev" | "test" | "bench" | "check")), ProfileChecking::LegacyRustc)
            // `cargo fix` and `cargo check` has legacy handling of this profile name
            | (Some(name @ "test"), ProfileChecking::LegacyTestOnly) => {
                if self._is_present("release") {
                    config.shell().warn(
                        "the `--release` flag should not be specified with the `--profile` flag\n\
                         The `--release` flag will be ignored.\n\
                         This was historically accepted, but will become an error \
                         in a future release."
                    )?;
                }
                return Ok(InternedString::new(name));
            }
            _ => {}
        }

        let conflict = |flag: &str, equiv: &str, specified: &str| -> anyhow::Error {
            anyhow::format_err!(
                "conflicting usage of --profile={} and --{flag}\n\
                 The `--{flag}` flag is the same as `--profile={equiv}`.\n\
                 Remove one flag or the other to continue.",
                specified,
                flag = flag,
                equiv = equiv
            )
        };

        let name = match (
            self.is_valid_and_present("release"),
            self.is_valid_and_present("debug"),
            specified_profile,
        ) {
            (false, false, None) => default,
            (true, _, None | Some("release")) => "release",
            (true, _, Some(name)) => return Err(conflict("release", "release", name)),
            (_, true, None | Some("dev")) => "dev",
            (_, true, Some(name)) => return Err(conflict("debug", "dev", name)),
            // `doc` is separate from all the other reservations because
            // [profile.doc] was historically allowed, but is deprecated and
            // has no effect. To avoid potentially breaking projects, it is a
            // warning in Cargo.toml, but since `--profile` is new, we can
            // reject it completely here.
            (_, _, Some("doc")) => {
                bail!("profile `doc` is reserved and not allowed to be explicitly specified")
            }
            (_, _, Some(name)) => {
                TomlProfile::validate_name(name)?;
                name
            }
        };

        Ok(InternedString::new(name))
    }

    fn packages_from_flags(&self) -> CargoResult<Packages> {
        Packages::from_flags(
            // TODO Integrate into 'workspace'
            self.is_valid_and_present("workspace") || self.is_valid_and_present("all"),
            self._is_valid_arg("exclude")
                .then(|| self._values_of("exclude"))
                .unwrap_or_default(),
            self._is_valid_arg("package")
                .then(|| self._values_of("package"))
                .unwrap_or_default(),
        )
    }

    fn compile_options(
        &self,
        config: &Config,
        mode: CompileMode,
        workspace: Option<&Workspace<'_>>,
        profile_checking: ProfileChecking,
    ) -> CargoResult<CompileOptions> {
        let spec = self.packages_from_flags()?;
        let mut message_format = None;
        let default_json = MessageFormat::Json {
            short: false,
            ansi: false,
            render_diagnostics: false,
        };
        for fmt in self._values_of("message-format") {
            for fmt in fmt.split(',') {
                let fmt = fmt.to_ascii_lowercase();
                match fmt.as_str() {
                    "json" => {
                        if message_format.is_some() {
                            bail!("cannot specify two kinds of `message-format` arguments");
                        }
                        message_format = Some(default_json);
                    }
                    "human" => {
                        if message_format.is_some() {
                            bail!("cannot specify two kinds of `message-format` arguments");
                        }
                        message_format = Some(MessageFormat::Human);
                    }
                    "short" => {
                        if message_format.is_some() {
                            bail!("cannot specify two kinds of `message-format` arguments");
                        }
                        message_format = Some(MessageFormat::Short);
                    }
                    "json-render-diagnostics" => {
                        if message_format.is_none() {
                            message_format = Some(default_json);
                        }
                        match &mut message_format {
                            Some(MessageFormat::Json {
                                render_diagnostics, ..
                            }) => *render_diagnostics = true,
                            _ => bail!("cannot specify two kinds of `message-format` arguments"),
                        }
                    }
                    "json-diagnostic-short" => {
                        if message_format.is_none() {
                            message_format = Some(default_json);
                        }
                        match &mut message_format {
                            Some(MessageFormat::Json { short, .. }) => *short = true,
                            _ => bail!("cannot specify two kinds of `message-format` arguments"),
                        }
                    }
                    "json-diagnostic-rendered-ansi" => {
                        if message_format.is_none() {
                            message_format = Some(default_json);
                        }
                        match &mut message_format {
                            Some(MessageFormat::Json { ansi, .. }) => *ansi = true,
                            _ => bail!("cannot specify two kinds of `message-format` arguments"),
                        }
                    }
                    s => bail!("invalid message format specifier: `{}`", s),
                }
            }
        }

        let mut build_config = BuildConfig::new(
            config,
            self.jobs()?,
            self.keep_going(),
            &self.targets(),
            mode,
        )?;
        build_config.message_format = message_format.unwrap_or(MessageFormat::Human);
        build_config.requested_profile = self.get_profile_name(config, "dev", profile_checking)?;
        build_config.build_plan = self.is_valid_and_present("build-plan");
        build_config.unit_graph = self.is_valid_and_present("unit-graph");
        build_config.future_incompat_report = self.is_valid_and_present("future-incompat-report");

        if self.is_valid_and_present("timings") {
            for timing_output in self._values_of("timings") {
                for timing_output in timing_output.split(',') {
                    let timing_output = timing_output.to_ascii_lowercase();
                    let timing_output = match timing_output.as_str() {
                        "html" => {
                            config
                                .cli_unstable()
                                .fail_if_stable_opt("--timings=html", 7405)?;
                            TimingOutput::Html
                        }
                        "json" => {
                            config
                                .cli_unstable()
                                .fail_if_stable_opt("--timings=json", 7405)?;
                            TimingOutput::Json
                        }
                        s => bail!("invalid timings output specifier: `{}`", s),
                    };
                    build_config.timing_outputs.push(timing_output);
                }
            }
            if build_config.timing_outputs.is_empty() {
                build_config.timing_outputs.push(TimingOutput::Html);
            }
        }

        if build_config.keep_going {
            config
                .cli_unstable()
                .fail_if_stable_opt("--keep-going", 10496)?;
        }
        if build_config.build_plan {
            config
                .cli_unstable()
                .fail_if_stable_opt("--build-plan", 5579)?;
        };
        if build_config.unit_graph {
            config
                .cli_unstable()
                .fail_if_stable_opt("--unit-graph", 8002)?;
        }

        let opts = CompileOptions {
            build_config,
            cli_features: self.cli_features()?,
            spec,
            filter: CompileFilter::from_raw_arguments(
                self.is_valid_and_present("lib"),
                self._values_of("bin"),
                self.is_valid_and_present("bins"),
                self._is_valid_arg("test")
                    .then(|| self._values_of("test"))
                    .unwrap_or_default(),
                self.is_valid_and_present("tests"),
                self._values_of("example"),
                self.is_valid_and_present("examples"),
                self._is_valid_arg("bench")
                    .then(|| self._values_of("bench"))
                    .unwrap_or_default(),
                self.is_valid_and_present("benches"),
                self.is_valid_and_present("all-targets"),
            ),
            target_rustdoc_args: None,
            target_rustc_args: None,
            target_rustc_crate_types: None,
            local_rustdoc_args: None,
            rustdoc_document_private_items: false,
            honor_rust_version: !self.is_valid_and_present("ignore-rust-version"),
        };

        if let Some(ws) = workspace {
            self.check_optional_opts(ws, &opts)?;
        } else if self._is_valid_arg("package") && self.is_present_with_zero_values("package") {
            // As for cargo 0.50.0, this won't occur but if someone sneaks in
            // we can still provide this informative message for them.
            anyhow::bail!(
                "\"--package <SPEC>\" requires a SPEC format value, \
                which can be any package ID specifier in the dependency graph.\n\
                Run `cargo help pkgid` for more information about SPEC format."
            )
        }

        Ok(opts)
    }

    fn cli_features(&self) -> CargoResult<CliFeatures> {
        CliFeatures::from_command_line(
            &self._values_of("features"),
            self._is_present("all-features"),
            !self._is_present("no-default-features"),
        )
    }

    fn compile_options_for_single_package(
        &self,
        config: &Config,
        mode: CompileMode,
        workspace: Option<&Workspace<'_>>,
        profile_checking: ProfileChecking,
    ) -> CargoResult<CompileOptions> {
        let mut compile_opts = self.compile_options(config, mode, workspace, profile_checking)?;
        let spec = self._values_of("package");
        if spec.iter().any(is_glob_pattern) {
            anyhow::bail!("Glob patterns on package selection are not supported.")
        }
        compile_opts.spec = Packages::Packages(spec);
        Ok(compile_opts)
    }

    fn new_options(&self, config: &Config) -> CargoResult<NewOptions> {
        let vcs = self._value_of("vcs").map(|vcs| match vcs {
            "git" => VersionControl::Git,
            "hg" => VersionControl::Hg,
            "pijul" => VersionControl::Pijul,
            "fossil" => VersionControl::Fossil,
            "none" => VersionControl::NoVcs,
            vcs => panic!("Impossible vcs: {:?}", vcs),
        });
        NewOptions::new(
            vcs,
            self._is_present("bin"),
            self._is_present("lib"),
            self.value_of_path("path", config).unwrap(),
            self._value_of("name").map(|s| s.to_string()),
            self._value_of("edition").map(|s| s.to_string()),
            self.registry(config)?,
        )
    }

    fn registry(&self, config: &Config) -> CargoResult<Option<String>> {
        match self._value_of("registry") {
            Some(registry) => {
                validate_package_name(registry, "registry name", "")?;

                if registry == CRATES_IO_REGISTRY {
                    // If "crates.io" is specified, then we just need to return `None`,
                    // as that will cause cargo to use crates.io. This is required
                    // for the case where a default alternative registry is used
                    // but the user wants to switch back to crates.io for a single
                    // command.
                    Ok(None)
                } else {
                    Ok(Some(registry.to_string()))
                }
            }
            None => config.default_registry(),
        }
    }

    fn index(&self) -> CargoResult<Option<String>> {
        let index = self._value_of("index").map(|s| s.to_string());
        Ok(index)
    }

    fn check_optional_opts(
        &self,
        workspace: &Workspace<'_>,
        compile_opts: &CompileOptions,
    ) -> CargoResult<()> {
        if self._is_valid_arg("package") && self.is_present_with_zero_values("package") {
            print_available_packages(workspace)?
        }

        if self.is_present_with_zero_values("example") {
            print_available_examples(workspace, compile_opts)?;
        }

        if self.is_present_with_zero_values("bin") {
            print_available_binaries(workspace, compile_opts)?;
        }

        if self._is_valid_arg("bench") && self.is_present_with_zero_values("bench") {
            print_available_benches(workspace, compile_opts)?;
        }

        if self._is_valid_arg("test") && self.is_present_with_zero_values("test") {
            print_available_tests(workspace, compile_opts)?;
        }

        Ok(())
    }

    fn is_present_with_zero_values(&self, name: &str) -> bool {
        self._is_present(name) && self._value_of(name).is_none()
    }

    fn is_valid_and_present(&self, name: &str) -> bool {
        self._is_valid_arg(name) && self._is_present(name)
    }

    fn _value_of(&self, name: &str) -> Option<&str>;

    fn _values_of(&self, name: &str) -> Vec<String>;

    fn _value_of_os(&self, name: &str) -> Option<&OsStr>;

    fn _values_of_os(&self, name: &str) -> Vec<OsString>;

    fn _occurrences(&self, name: &str) -> u32;

    fn _is_present(&self, name: &str) -> bool;

    fn _is_valid_arg(&self, name: &str) -> bool;
}

impl<'a> ArgMatchesExt for ArgMatches {
    fn _value_of(&self, name: &str) -> Option<&str> {
        self.value_of(name)
    }

    fn _value_of_os(&self, name: &str) -> Option<&OsStr> {
        self.value_of_os(name)
    }

    fn _values_of(&self, name: &str) -> Vec<String> {
        self.values_of(name)
            .unwrap_or_default()
            .map(|s| s.to_string())
            .collect()
    }

    fn _values_of_os(&self, name: &str) -> Vec<OsString> {
        self.values_of_os(name)
            .unwrap_or_default()
            .map(|s| s.to_os_string())
            .collect()
    }

    fn _occurrences(&self, name: &str) -> u32 {
        self.occurrences_of(name) as u32
    }

    fn _is_present(&self, name: &str) -> bool {
        self.is_present(name)
    }

    fn _is_valid_arg(&self, name: &str) -> bool {
        self.is_valid_arg(name)
    }
}

pub fn values(args: &ArgMatches, name: &str) -> Vec<String> {
    args._values_of(name)
}

pub fn values_os(args: &ArgMatches, name: &str) -> Vec<OsString> {
    args._values_of_os(name)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandInfo {
    BuiltIn { about: Option<String> },
    External { path: PathBuf },
    Alias { target: StringOrVec },
}
