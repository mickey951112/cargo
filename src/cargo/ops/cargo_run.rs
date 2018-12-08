use std::iter;
use std::path::Path;

use crate::core::{nightly_features_allowed, TargetKind, Workspace};
use crate::ops;
use crate::util::{self, CargoResult, ProcessError};

pub fn run(
    ws: &Workspace,
    options: &ops::CompileOptions,
    args: &[String],
) -> CargoResult<Option<ProcessError>> {
    let config = ws.config();

    // We compute the `bins` here *just for diagnosis*.  The actual set of
    // packages to be run is determined by the `ops::compile` call below.
    let packages = options.spec.get_packages(ws)?;
    let bins: Vec<_> = packages
        .into_iter()
        .flat_map(|pkg| {
            iter::repeat(pkg).zip(pkg.manifest().targets().iter().filter(|target| {
                !target.is_lib()
                    && !target.is_custom_build()
                    && if !options.filter.is_specific() {
                        target.is_bin()
                    } else {
                        options.filter.target_run(target)
                    }
            }))
        })
        .collect();

    if bins.is_empty() {
        if !options.filter.is_specific() {
            bail!("a bin target must be available for `cargo run`")
        } else {
            // this will be verified in cargo_compile
        }
    }

    if bins.len() == 1 {
        let target = bins[0].1;
        if let TargetKind::ExampleLib(..) = target.kind() {
            bail!(
                "example target `{}` is a library and cannot be executed",
                target.name()
            )
        }
    }

    if bins.len() > 1 {
        if !options.filter.is_specific() {
            let names: Vec<&str> = bins
                .into_iter()
                .map(|(_pkg, target)| target.name())
                .collect();
            if nightly_features_allowed() {
                bail!(
                    "`cargo run` could not determine which binary to run. \
                     Use the `--bin` option to specify a binary, \
                     or (on nightly) the `default-run` manifest key.\n\
                     available binaries: {}",
                    names.join(", ")
                )
            } else {
                bail!(
                    "`cargo run` requires that a package only have one \
                     executable; use the `--bin` option to specify which one \
                     to run\navailable binaries: {}",
                    names.join(", ")
                )
            }
        } else {
            bail!(
                "`cargo run` can run at most one executable, but \
                 multiple were specified"
            )
        }
    }

    let compile = ops::compile(ws, options)?;
    assert_eq!(compile.binaries.len(), 1);
    let exe = &compile.binaries[0];
    let exe = match util::without_prefix(exe, config.cwd()) {
        Some(path) if path.file_name() == Some(path.as_os_str()) => {
            Path::new(".").join(path).to_path_buf()
        }
        Some(path) => path.to_path_buf(),
        None => exe.to_path_buf(),
    };
    let pkg = bins[0].0;
    let mut process = compile.target_process(exe, pkg)?;
    process.args(args).cwd(config.cwd());

    config.shell().status("Running", process.to_string())?;

    let result = process.exec_replace();

    match result {
        Ok(()) => Ok(None),
        Err(e) => {
            let err = e.downcast::<ProcessError>()?;
            Ok(Some(err))
        }
    }
}
