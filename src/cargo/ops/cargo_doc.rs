use crate::core::compiler::RustcTargetData;
use crate::core::resolver::HasDevUnits;
use crate::core::{Shell, Workspace};
use crate::ops;
use crate::util::CargoResult;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// Strongly typed options for the `cargo doc` command.
#[derive(Debug)]
pub struct DocOptions {
    /// Whether to attempt to open the browser after compiling the docs
    pub open_result: bool,
    /// Options to pass through to the compiler
    pub compile_opts: ops::CompileOptions,
}

#[derive(Deserialize)]
struct CargoDocConfig {
    /// Browser to use to open docs. If this is unset, the value of the environment variable
    /// `BROWSER` will be used.
    browser: Option<String>,
}

/// Main method for `cargo doc`.
pub fn doc(ws: &Workspace<'_>, options: &DocOptions) -> CargoResult<()> {
    let specs = options.compile_opts.spec.to_package_id_specs(ws)?;
    let target_data = RustcTargetData::new(ws, &options.compile_opts.build_config.requested_kinds)?;
    let ws_resolve = ops::resolve_ws_with_opts(
        ws,
        &target_data,
        &options.compile_opts.build_config.requested_kinds,
        &options.compile_opts.cli_features,
        &specs,
        HasDevUnits::No,
        crate::core::resolver::features::ForceAllTargets::No,
    )?;

    let ids = ws_resolve.targeted_resolve.specs_to_ids(&specs)?;
    let pkgs = ws_resolve.pkg_set.get_many(ids)?;

    let mut lib_names = HashMap::new();
    let mut bin_names = HashMap::new();
    let mut names = Vec::new();
    for package in &pkgs {
        for target in package.targets().iter().filter(|t| t.documented()) {
            if target.is_lib() {
                if let Some(prev) = lib_names.insert(target.crate_name(), package) {
                    anyhow::bail!(
                        "The library `{}` is specified by packages `{}` and \
                         `{}` but can only be documented once. Consider renaming \
                         or marking one of the targets as `doc = false`.",
                        target.crate_name(),
                        prev,
                        package
                    );
                }
            } else if let Some(prev) = bin_names.insert(target.crate_name(), package) {
                anyhow::bail!(
                    "The binary `{}` is specified by packages `{}` and \
                     `{}` but can be documented only once. Consider renaming \
                     or marking one of the targets as `doc = false`.",
                    target.crate_name(),
                    prev,
                    package
                );
            }
            names.push(target.crate_name());
        }
    }

    let open_kind = if options.open_result {
        Some(options.compile_opts.build_config.single_requested_kind()?)
    } else {
        None
    };

    let compilation = ops::compile(ws, &options.compile_opts)?;

    if let Some(kind) = open_kind {
        let name = match names.first() {
            Some(s) => s.to_string(),
            None => return Ok(()),
        };
        let path = compilation.root_output[&kind]
            .with_file_name("doc")
            .join(&name)
            .join("index.html");
        if path.exists() {
            let mut shell = ws.config().shell();
            shell.status("Opening", path.display())?;
            let cfg = ws.config().get::<CargoDocConfig>("cargo-doc")?;
            open_docs(&path, &mut shell, cfg.browser.map(|v| v.into()))?;
        }
    }

    Ok(())
}

fn open_docs(path: &Path, shell: &mut Shell, config_browser: Option<OsString>) -> CargoResult<()> {
    let browser = config_browser.or_else(|| std::env::var_os("BROWSER"));
    match browser {
        Some(browser) => {
            if let Err(e) = Command::new(&browser).arg(path).status() {
                shell.warn(format!(
                    "Couldn't open docs with {}: {}",
                    browser.to_string_lossy(),
                    e
                ))?;
            }
        }
        None => {
            if let Err(e) = opener::open(&path) {
                let e = e.into();
                crate::display_warning_with_error("couldn't open docs", &e, shell);
            }
        }
    };

    Ok(())
}
