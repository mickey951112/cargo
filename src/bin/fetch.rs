use cargo::core::Workspace;
use cargo::ops;
use cargo::util::{CliResult, Config};
use cargo::util::important_paths::find_root_manifest_for_wd;

#[derive(RustcDecodable)]
pub struct Options {
    flag_manifest_path: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
}

pub const USAGE: &'static str = "
Fetch dependencies of a package from the network.

Usage:
    cargo fetch [options]

Options:
    -h, --help               Print this message
    --manifest-path PATH     Path to the manifest to fetch dependencies for
    -v, --verbose ...        Use verbose output
    -q, --quiet              No output printed to stdout
    --color WHEN             Coloring: auto, always, never

If a lockfile is available, this command will ensure that all of the git
dependencies and/or registries dependencies are downloaded and locally
available. The network is never touched after a `cargo fetch` unless
the lockfile changes.

If the lockfile is not available, then this is the equivalent of
`cargo generate-lockfile`. A lockfile is generated and dependencies are also
all updated.
";

pub fn execute(options: Options, config: &Config) -> CliResult<Option<()>> {
    try!(config.configure_shell(options.flag_verbose,
                                options.flag_quiet,
                                &options.flag_color));
    let root = try!(find_root_manifest_for_wd(options.flag_manifest_path, config.cwd()));
    let ws = try!(Workspace::new(&root, config));
    try!(ops::fetch(&ws));
    Ok(None)
}

