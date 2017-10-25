use std::env;

use cargo::core::Workspace;
use cargo::ops;
use cargo::util::{CliResult, Config};
use cargo::util::important_paths::find_root_manifest_for_wd;

#[derive(Deserialize)]
pub struct Options {
    flag_package: Vec<String>,
    flag_aggressive: bool,
    flag_precise: Option<String>,
    flag_manifest_path: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
    flag_frozen: bool,
    flag_locked: bool,
    #[serde(rename = "flag_Z")]
    flag_z: Vec<String>,
}

pub const USAGE: &'static str = "
Update dependencies as recorded in the local lock file.

Usage:
    cargo update [options]

Options:
    -h, --help                   Print this message
    -p SPEC, --package SPEC ...  Package to update
    --aggressive                 Force updating all dependencies of <name> as well
    --precise PRECISE            Update a single dependency to exactly PRECISE
    --manifest-path PATH         Path to the crate's manifest
    -v, --verbose ...            Use verbose output (-vv very verbose/build.rs output)
    -q, --quiet                  No output printed to stdout
    --color WHEN                 Coloring: auto, always, never
    --frozen                     Require Cargo.lock and cache are up to date
    --locked                     Require Cargo.lock is up to date
    -Z FLAG ...                  Unstable (nightly-only) flags to Cargo

This command requires that a `Cargo.lock` already exists as generated by
`cargo build` or related commands.

If SPEC is given, then a conservative update of the lockfile will be
performed. This means that only the dependency specified by SPEC will be
updated. Its transitive dependencies will be updated only if SPEC cannot be
updated without updating dependencies.  All other dependencies will remain
locked at their currently recorded versions.

If PRECISE is specified, then --aggressive must not also be specified. The
argument PRECISE is a string representing a precise revision that the package
being updated should be updated to. For example, if the package comes from a git
repository, then PRECISE would be the exact revision that the repository should
be updated to.

If SPEC is not given, then all dependencies will be re-resolved and
updated.

For more information about package id specifications, see `cargo help pkgid`.
";

pub fn execute(options: Options, config: &mut Config) -> CliResult {
    debug!("executing; cmd=cargo-update; args={:?}", env::args().collect::<Vec<_>>());
    config.configure(options.flag_verbose,
                     options.flag_quiet,
                     &options.flag_color,
                     options.flag_frozen,
                     options.flag_locked,
                     &options.flag_z)?;
    let root = find_root_manifest_for_wd(options.flag_manifest_path, config.cwd())?;

    let update_opts = ops::UpdateOptions {
        aggressive: options.flag_aggressive,
        precise: options.flag_precise.as_ref().map(|s| &s[..]),
        to_update: &options.flag_package,
        config: config,
    };

    let ws = Workspace::new(&root, config)?;
    ops::update_lockfile(&ws, &update_opts)?;
    Ok(())
}
