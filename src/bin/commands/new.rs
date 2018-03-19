use command_prelude::*;

use cargo::ops;

pub fn cli() -> App {
    subcommand("new")
        .about("Create a new cargo package at <path>")
        .arg(Arg::with_name("path").required(true))
        .arg_new_opts()
}

pub fn exec(config: &mut Config, args: &ArgMatches) -> CliResult {
    let opts = args.new_options(config)?;
    ops::new(&opts, config)?;
    let path = args.value_of("path").unwrap();
    config
        .shell()
        .status("Created", format!("{} `{}` project", opts.kind, path))?;
    Ok(())
}
