use command_prelude::*;

use cargo::ops::{self, CompileMode, TestOptions};

pub fn cli() -> App {
    subcommand("bench")
        .setting(AppSettings::TrailingVarArg)
        .about("Execute all benchmarks of a local package")
        .arg(
            Arg::with_name("BENCHNAME")
                .help("If specified, only run benches containing this string in their names"),
        )
        .arg(
            Arg::with_name("args")
                .help("Arguments for the bench binary")
                .multiple(true)
                .last(true),
        )
        .arg_targets_all(
            "Benchmark only this package's library",
            "Benchmark only the specified binary",
            "Benchmark all binaries",
            "Benchmark only the specified example",
            "Benchmark all examples",
            "Benchmark only the specified test target",
            "Benchmark all tests",
            "Benchmark only the specified bench target",
            "Benchmark all benches",
            "Benchmark all targets (default)",
        )
        .arg(opt("no-run", "Compile, but don't run benchmarks"))
        .arg_package(
            "Package to run benchmarks for",
            "Benchmark all packages in the workspace",
            "Exclude packages from the benchmark",
        )
        .arg_jobs()
        .arg_features()
        .arg_target_triple("Build for the target triple")
        .arg_manifest_path()
        .arg_message_format()
        .arg(opt(
            "no-fail-fast",
            "Run all benchmarks regardless of failure",
        ))
        .after_help(
            "\
All of the trailing arguments are passed to the benchmark binaries generated
for filtering benchmarks and generally providing options configuring how they
run.

If the --package argument is given, then SPEC is a package id specification
which indicates which package should be benchmarked. If it is not given, then
the current package is benchmarked. For more information on SPEC and its format,
see the `cargo help pkgid` command.

All packages in the workspace are benchmarked if the `--all` flag is supplied. The
`--all` flag is automatically assumed for a virtual manifest.
Note that `--exclude` has to be specified in conjunction with the `--all` flag.

The --jobs argument affects the building of the benchmark executable but does
not affect how many jobs are used when running the benchmarks.

Compilation can be customized with the `bench` profile in the manifest.
",
        )
}

pub fn exec(config: &mut Config, args: &ArgMatches) -> CliResult {
    let ws = args.workspace(config)?;
    let mut compile_opts = args.compile_options(config, CompileMode::Bench)?;
    compile_opts.release = true;

    let ops = TestOptions {
        no_run: args.is_present("no-run"),
        no_fail_fast: args.is_present("no-fail-fast"),
        only_doc: false,
        compile_opts,
    };

    let mut bench_args = vec![];
    bench_args.extend(
        args.value_of("BENCHNAME")
            .into_iter()
            .map(|s| s.to_string()),
    );
    bench_args.extend(
        args.values_of("args")
            .unwrap_or_default()
            .map(|s| s.to_string()),
    );

    let err = ops::run_benches(&ws, &ops, &bench_args)?;
    match err {
        None => Ok(()),
        Some(err) => Err(match err.exit.as_ref().and_then(|e| e.code()) {
            Some(i) => CliError::new(format_err!("bench failed"), i),
            None => CliError::new(err.into(), 101),
        }),
    }
}
