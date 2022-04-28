use crate::command_prelude::*;

use cargo::core::{GitReference, SourceId, Workspace};
use cargo::ops;
use cargo::util::IntoUrl;

use cargo_util::paths;

pub fn cli() -> App {
    subcommand("install")
        .about("Install a Rust binary. Default location is $HOME/.cargo/bin")
        .arg_quiet()
        .arg(
            Arg::new("crate")
                .forbid_empty_values(true)
                .multiple_values(true),
        )
        .arg(
            opt("version", "Specify a version to install")
                .alias("vers")
                .value_name("VERSION")
                .requires("crate"),
        )
        .arg(
            opt("git", "Git URL to install the specified crate from")
                .value_name("URL")
                .conflicts_with_all(&["path", "index", "registry"]),
        )
        .arg(
            opt("branch", "Branch to use when installing from git")
                .value_name("BRANCH")
                .requires("git"),
        )
        .arg(
            opt("tag", "Tag to use when installing from git")
                .value_name("TAG")
                .requires("git"),
        )
        .arg(
            opt("rev", "Specific commit to use when installing from git")
                .value_name("SHA")
                .requires("git"),
        )
        .arg(
            opt("path", "Filesystem path to local crate to install")
                .value_name("PATH")
                .conflicts_with_all(&["git", "index", "registry"]),
        )
        .arg(opt(
            "list",
            "list all installed packages and their versions",
        ))
        .arg_jobs()
        .arg(opt("force", "Force overwriting existing crates or binaries").short('f'))
        .arg(opt("no-track", "Do not save tracking information"))
        .arg_features()
        .arg_profile("Install artifacts with the specified profile")
        .arg(opt("debug", "Build in debug mode instead of release mode"))
        .arg_targets_bins_examples(
            "Install only the specified binary",
            "Install all binaries",
            "Install only the specified example",
            "Install all examples",
        )
        .arg_target_triple("Build for the target triple")
        .arg_target_dir()
        .arg(opt("root", "Directory to install packages into").value_name("DIR"))
        .arg(
            opt("index", "Registry index to install from")
                .value_name("INDEX")
                .requires("crate")
                .conflicts_with_all(&["git", "path", "registry"]),
        )
        .arg(
            opt("registry", "Registry to use")
                .value_name("REGISTRY")
                .requires("crate")
                .conflicts_with_all(&["git", "path", "index"]),
        )
        .arg_message_format()
        .arg_timings()
        .after_help("Run `cargo help install` for more detailed information.\n")
}

pub fn exec(config: &mut Config, args: &ArgMatches) -> CliResult {
    let path = args.value_of_path("path", config);
    if let Some(path) = &path {
        config.reload_rooted_at(path)?;
    } else {
        // TODO: Consider calling set_search_stop_path(home).
        config.reload_rooted_at(config.home().clone().into_path_unlocked())?;
    }

    // In general, we try to avoid normalizing paths in Cargo,
    // but in these particular cases we need it to fix rust-lang/cargo#10283.
    // (Handle `SourceId::for_path` and `Workspace::new`,
    // but not `Config::reload_rooted_at` which is always cwd)
    let path = path.map(|p| paths::normalize_path(&p));

    let version = args.value_of("version");
    let krates = args
        .values_of("crate")
        .unwrap_or_default()
        .map(|k| (k, version))
        .collect::<Vec<_>>();

    let mut from_cwd = false;

    let source = if let Some(url) = args.value_of("git") {
        let url = url.into_url()?;
        let gitref = if let Some(branch) = args.value_of("branch") {
            GitReference::Branch(branch.to_string())
        } else if let Some(tag) = args.value_of("tag") {
            GitReference::Tag(tag.to_string())
        } else if let Some(rev) = args.value_of("rev") {
            GitReference::Rev(rev.to_string())
        } else {
            GitReference::DefaultBranch
        };
        SourceId::for_git(&url, gitref)?
    } else if let Some(path) = &path {
        SourceId::for_path(path)?
    } else if krates.is_empty() {
        from_cwd = true;
        SourceId::for_path(config.cwd())?
    } else if let Some(registry) = args.registry(config)? {
        SourceId::alt_registry(config, &registry)?
    } else if let Some(index) = args.value_of("index") {
        SourceId::for_registry(&index.into_url()?)?
    } else {
        SourceId::crates_io(config)?
    };

    let root = args.value_of("root");

    // We only provide workspace information for local crate installation from
    // one of the following sources:
    // - From current working directory (only work for edition 2015).
    // - From a specific local file path (from `--path` arg).
    //
    // This workspace information is for emitting helpful messages from
    // `ArgMatchesExt::compile_options` and won't affect the actual compilation.
    let workspace = if from_cwd {
        args.workspace(config).ok()
    } else if let Some(path) = &path {
        Workspace::new(&path.join("Cargo.toml"), config).ok()
    } else {
        None
    };

    let mut compile_opts = args.compile_options(
        config,
        CompileMode::Build,
        workspace.as_ref(),
        ProfileChecking::Custom,
    )?;

    compile_opts.build_config.requested_profile =
        args.get_profile_name(config, "release", ProfileChecking::Custom)?;

    if args.is_present("list") {
        ops::install_list(root, config)?;
    } else {
        ops::install(
            config,
            root,
            krates,
            source,
            from_cwd,
            &compile_opts,
            args.is_present("force"),
            args.is_present("no-track"),
        )?;
    }
    Ok(())
}
