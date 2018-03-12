use command_prelude::*;

use cargo::ops;

pub fn cli() -> App {
    subcommand("init")
        .about("Create a new cargo package in an existing directory")
        .arg(Arg::with_name("path").default_value("."))
        .arg_new_opts()
}

pub fn exec(config: &mut Config, args: &ArgMatches) -> CliResult {
    let opts = args.new_options()?;
    ops::init(&opts, config)?;
    config.shell().status("Created", format!("{} project", opts.kind))?;
    Ok(())
}
