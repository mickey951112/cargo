use command_prelude::*;

use cargo::ops;

pub fn cli() -> App {
    subcommand("uninstall")
        .about("Remove a Rust binary")
        .arg(Arg::with_name("spec").multiple(true))
        .arg(
            opt("bin", "Only uninstall the binary NAME")
                .value_name("NAME").multiple(true)
        )
        .arg(
            opt("root", "Directory to uninstall packages from")
                .value_name("DIR")
        )
        .after_help("\
The argument SPEC is a package id specification (see `cargo help pkgid`) to
specify which crate should be uninstalled. By default all binaries are
uninstalled for a crate but the `--bin` and `--example` flags can be used to
only uninstall particular binaries.
")
}

pub fn exec(config: &mut Config, args: &ArgMatches) -> CliResult {
    let root = args.value_of("root");
    let specs = args.values_of("spec").unwrap_or_default().collect();
    ops::uninstall(root, specs, &values(args, "bin"), config)?;
    Ok(())
}
