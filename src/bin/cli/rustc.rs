use clap::AppSettings;

use super::utils::*;

pub fn cli() -> App {
    subcommand("rustc")
        .setting(AppSettings::TrailingVarArg)
        .about("Compile a package and all of its dependencies")
        .arg(Arg::with_name("args").multiple(true))
        .arg(
            opt("package", "Package to build")
                .short("p").value_name("SPEC")
        )
        .arg_jobs()
        .arg_targets_all(
            "Build only this package's library",
            "Build only the specified binary",
            "Build all binaries",
            "Build only the specified example",
            "Build all examples",
            "Build only the specified test target",
            "Build all tests",
            "Build only the specified bench target",
            "Build all benches",
            "Build all targets (lib and bin targets by default)",
        )
        .arg_release("Build artifacts in release mode, with optimizations")
        .arg(
            opt("profile", "Profile to build the selected target for")
                .value_name("PROFILE")
        )
        .arg_features()
        .arg_target_triple("Target triple which compiles will be for")
        .arg_manifest_path()
        .arg_message_format()
        .after_help("\
The specified target for the current package (or package specified by SPEC if
provided) will be compiled along with all of its dependencies. The specified
<args>... will all be passed to the final compiler invocation, not any of the
dependencies. Note that the compiler will still unconditionally receive
arguments such as -L, --extern, and --crate-type, and the specified <args>...
will simply be added to the compiler invocation.

This command requires that only one target is being compiled. If more than one
target is available for the current package the filters of --lib, --bin, etc,
must be used to select which target is compiled. To pass flags to all compiler
processes spawned by Cargo, use the $RUSTFLAGS environment variable or the
`build.rustflags` configuration option.
")
}
