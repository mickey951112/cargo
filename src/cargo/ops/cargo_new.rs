use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io;
use std::path::Path;

use rustc_serialize::{Decodable, Decoder};

use git2::Config as GitConfig;

use util::{GitRepo, HgRepo, CargoResult, human, ChainError, internal};
use util::Config;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VersionControl { Git, Hg, NoVcs }

pub struct NewOptions<'a> {
    pub version_control: Option<VersionControl>,
    pub bin: bool,
    pub path: &'a str,
}

impl Decodable for VersionControl {
    fn decode<D: Decoder>(d: &mut D) -> Result<VersionControl, D::Error> {
        Ok(match &try!(d.read_str())[..] {
            "git" => VersionControl::Git,
            "hg" => VersionControl::Hg,
            "none" => VersionControl::NoVcs,
            n => {
                let err = format!("could not decode '{}' as version control", n);
                return Err(d.error(&err));
            }
        })
    }
}

struct CargoNewConfig {
    name: Option<String>,
    email: Option<String>,
    version_control: Option<VersionControl>,
}

pub fn new(opts: NewOptions, config: &Config) -> CargoResult<()> {
    let path = config.cwd().join(opts.path);
    if fs::metadata(&path).is_ok() {
        return Err(human(format!("Destination `{}` already exists",
                                 path.display())))
    }
    let name = try!(path.file_name().and_then(|s| s.to_str()).chain_error(|| {
        human(&format!("cannot create a project with a non-unicode name: {:?}",
                       path.file_name().unwrap()))
    }));
    for c in name.chars() {
        if c.is_alphanumeric() { continue }
        if c == '_' || c == '-' { continue }
        return Err(human(&format!("Invalid character `{}` in crate name: `{}`",
                                  c, name)));
    }
    mk(config, &path, name, &opts).chain_error(|| {
        human(format!("Failed to create project `{}` at `{}`",
                      name, path.display()))
    })
}

fn existing_vcs_repo(path: &Path) -> bool {
    GitRepo::discover(path).is_ok() || HgRepo::discover(path).is_ok()
}

fn file(p: &Path, contents: &[u8]) -> io::Result<()> {
    try!(File::create(p)).write_all(contents)
}

fn mk(config: &Config, path: &Path, name: &str,
      opts: &NewOptions) -> CargoResult<()> {
    let cfg = try!(global_config(config));
    let mut ignore = "target\n".to_string();
    let in_existing_vcs_repo = existing_vcs_repo(path.parent().unwrap());
    if !opts.bin {
        ignore.push_str("Cargo.lock\n");
    }

    let vcs = match (opts.version_control, cfg.version_control, in_existing_vcs_repo) {
        (None, None, false) => VersionControl::Git,
        (None, Some(option), false) => option,
        (Some(option), _, false) => option,
        (_, _, true) => VersionControl::NoVcs,
    };

    match vcs {
        VersionControl::Git => {
            try!(GitRepo::init(path));
            try!(file(&path.join(".gitignore"), ignore.as_bytes()));
        },
        VersionControl::Hg => {
            try!(HgRepo::init(path));
            try!(file(&path.join(".hgignore"), ignore.as_bytes()));
        },
        VersionControl::NoVcs => {
            try!(fs::create_dir(path));
        },
    };

    let (author_name, email) = try!(discover_author());
    // Hoo boy, sure glad we've got exhaustivenes checking behind us.
    let author = match (cfg.name, cfg.email, author_name, email) {
        (Some(name), Some(email), _, _) |
        (Some(name), None, _, Some(email)) |
        (None, Some(email), name, _) |
        (None, None, name, Some(email)) => format!("{} <{}>", name, email),
        (Some(name), None, _, None) |
        (None, None, name, None) => name,
    };

    try!(file(&path.join("Cargo.toml"), format!(
r#"[package]
name = "{}"
version = "0.0.1"
authors = ["{}"]
"#, name, author).as_bytes()));

    try!(fs::create_dir(&path.join("src")));

    if opts.bin {
        try!(file(&path.join("src/main.rs"), b"\
fn main() {
    println!(\"Hello, world!\");
}
"));
    } else {
        try!(file(&path.join("src/lib.rs"), b"\
#[test]
fn it_works() {
}
"));
    }

    Ok(())
}

fn discover_author() -> CargoResult<(String, Option<String>)> {
    let git_config = GitConfig::open_default().ok();
    let git_config = git_config.as_ref();
    let name = git_config.and_then(|g| g.get_string("user.name").ok())
                         .map(|s| s.to_string())
                         .or_else(|| env::var("USER").ok())      // unix
                         .or_else(|| env::var("USERNAME").ok()); // windows
    let name = match name {
        Some(name) => name,
        None => {
            let username_var = if cfg!(windows) {"USERNAME"} else {"USER"};
            return Err(human(format!("could not determine the current \
                                      user, please set ${}", username_var)))
        }
    };
    let email = git_config.and_then(|g| g.get_string("user.email").ok());

    let name = name.trim().to_string();
    let email = email.map(|s| s.trim().to_string());

    Ok((name, email))
}

fn global_config(config: &Config) -> CargoResult<CargoNewConfig> {
    let name = try!(config.get_string("cargo-new.name")).map(|s| s.0);
    let email = try!(config.get_string("cargo-new.email")).map(|s| s.0);
    let vcs = try!(config.get_string("cargo-new.vcs"));

    let vcs = match vcs.as_ref().map(|p| (&p.0[..], &p.1)) {
        Some(("git", _)) => Some(VersionControl::Git),
        Some(("hg", _)) => Some(VersionControl::Hg),
        Some(("none", _)) => Some(VersionControl::NoVcs),
        Some((s, p)) => {
            return Err(internal(format!("invalid configuration for key \
                                         `cargo-new.vcs`, unknown vcs `{}` \
                                         (found in {:?})", s, p)))
        }
        None => None
    };
    Ok(CargoNewConfig {
        name: name,
        email: email,
        version_control: vcs,
    })
}
