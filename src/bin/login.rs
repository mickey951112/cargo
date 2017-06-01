use std::io::prelude::*;
use std::io;

use cargo::ops;
use cargo::core::{SourceId, Source};
use cargo::sources::RegistrySource;
use cargo::util::{CliResult, CargoResultExt, Config};

#[derive(RustcDecodable)]
pub struct Options {
    flag_host: Option<String>,
    arg_token: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
    flag_frozen: bool,
    flag_locked: bool,
}

pub const USAGE: &'static str = "
Save an api token from the registry locally

Usage:
    cargo login [options] [<token>]

Options:
    -h, --help               Print this message
    --host HOST              Host to set the token for
    -v, --verbose ...        Use verbose output (-vv very verbose/build.rs output)
    -q, --quiet              No output printed to stdout
    --color WHEN             Coloring: auto, always, never
    --frozen                 Require Cargo.lock and cache are up to date
    --locked                 Require Cargo.lock is up to date

";

pub fn execute(options: Options, config: &Config) -> CliResult {
    config.configure(options.flag_verbose,
                     options.flag_quiet,
                     &options.flag_color,
                     options.flag_frozen,
                     options.flag_locked)?;

    let host = match options.flag_host {
        Some(host) => host,
        None => {
            let src = SourceId::crates_io(config)?;
            let mut src = RegistrySource::remote(&src, config);
            src.update()?;
            src.config()?.unwrap().api
        }
    };

    let token = match options.arg_token {
        Some(token) => token,
        None => {
            println!("please visit {}me and paste the API Token below", &host);
            let mut line = String::new();
            let input = io::stdin();
            input.lock().read_line(&mut line).chain_err(|| {
                "failed to read stdin"
            })?;
            line.trim().to_string()
        }
    };

    ops::registry_login(config, token, host)?;
    Ok(())
}
