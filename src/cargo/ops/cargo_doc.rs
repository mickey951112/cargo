use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use core::{PackageIdSpec, Workspace};
use ops;
use util::CargoResult;

pub struct DocOptions<'a> {
    pub open_result: bool,
    pub compile_opts: ops::CompileOptions<'a>,
}

pub fn doc(ws: &Workspace, options: &DocOptions) -> CargoResult<()> {
    let package = ws.current()?;

    let spec = match options.compile_opts.spec {
        ops::Packages::Packages(packages) => packages,
        _ => {
            // This should not happen, because the `doc` binary is hard-coded to pass
            // the `Packages::Packages` variant.
            bail!("`cargo doc` does not support the `--all` flag")
        },
    };

    let mut lib_names = HashSet::new();
    let mut bin_names = HashSet::new();
    if spec.is_empty() {
        for target in package.targets().iter().filter(|t| t.documented()) {
            if target.is_lib() {
                assert!(lib_names.insert(target.crate_name()));
            } else {
                assert!(bin_names.insert(target.crate_name()));
            }
        }
        for bin in bin_names.iter() {
            if lib_names.contains(bin) {
                bail!("cannot document a package where a library and a binary \
                       have the same name. Consider renaming one or marking \
                       the target as `doc = false`")
            }
        }
    }

    ops::compile(ws, &options.compile_opts)?;

    if options.open_result {
        let name = if spec.len() > 1 {
            bail!("Passing multiple packages and `open` is not supported")
        } else if spec.len() == 1 {
            PackageIdSpec::parse(&spec[0])?
                .name()
                .replace("-", "_")
        } else {
            match lib_names.iter().chain(bin_names.iter()).nth(0) {
                Some(s) => s.to_string(),
                None => return Ok(()),
            }
        };

        // Don't bother locking here as if this is getting deleted there's
        // nothing we can do about it and otherwise if it's getting overwritten
        // then that's also ok!
        let mut target_dir = ws.target_dir();
        if let Some(triple) = options.compile_opts.target {
            target_dir.push(Path::new(triple).file_stem().unwrap());
        }
        let path = target_dir.join("doc").join(&name).join("index.html");
        let path = path.into_path_unlocked();
        if fs::metadata(&path).is_ok() {
            let mut shell = options.compile_opts.config.shell();
            shell.status("Opening", path.display())?;
            match open_docs(&path) {
                Ok(m) => shell.status("Launching", m)?,
                Err(e) => {
                    shell.warn(
                            "warning: could not determine a browser to open docs with, tried:")?;
                    for method in e {
                        shell.warn(format!("\t{}", method))?;
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_docs(path: &Path) -> Result<&'static str, Vec<&'static str>> {
    use std::env;
    let mut methods = Vec::new();
    // trying $BROWSER
    if let Ok(name) = env::var("BROWSER") {
        match Command::new(name).arg(path).status() {
            Ok(_) => return Ok("$BROWSER"),
            Err(_) => methods.push("$BROWSER"),
        }
    }

    for m in ["xdg-open", "gnome-open", "kde-open"].iter() {
        match Command::new(m).arg(path).status() {
            Ok(_) => return Ok(m),
            Err(_) => methods.push(m),
        }
    }

    Err(methods)
}

#[cfg(target_os = "windows")]
fn open_docs(path: &Path) -> Result<&'static str, Vec<&'static str>> {
    match Command::new("cmd").arg("/C").arg(path).status() {
        Ok(_) => Ok("cmd /C"),
        Err(_) => Err(vec!["cmd /C"]),
    }
}

#[cfg(target_os = "macos")]
fn open_docs(path: &Path) -> Result<&'static str, Vec<&'static str>> {
    match Command::new("open").arg(path).status() {
        Ok(_) => Ok("open"),
        Err(_) => Err(vec!["open"]),
    }
}
