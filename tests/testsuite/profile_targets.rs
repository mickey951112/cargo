use support::is_nightly;
use support::{basic_manifest, execs, project, Project};
use support::hamcrest::assert_that;

// These tests try to exercise exactly which profiles are selected for every
// target.

fn all_target_project() -> Project {
    // This abuses the `codegen-units` setting so that we can verify exactly
    // which profile is used for each compiler invocation.
    project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"

            [dependencies]
            bar = { path = "bar" }

            [build-dependencies]
            bdep = { path = "bdep" }

            [profile.dev]
            codegen-units = 1
            panic = "abort"
            [profile.release]
            codegen-units = 2
            panic = "abort"
            [profile.test]
            codegen-units = 3
            [profile.bench]
            codegen-units = 4
        "#,
        )
        .file("src/lib.rs", "extern crate bar;")
        .file("src/main.rs", r#"
            extern crate foo;
            fn main() {}
        "#)
        .file("examples/ex1.rs", r#"
            extern crate foo;
            fn main() {}
        "#)
        .file("tests/test1.rs", "extern crate foo;")
        .file("benches/bench1.rs", "extern crate foo;")
        .file("build.rs", r#"
            extern crate bdep;
            fn main() {
                eprintln!("foo custom build PROFILE={} DEBUG={} OPT_LEVEL={}",
                    std::env::var("PROFILE").unwrap(),
                    std::env::var("DEBUG").unwrap(),
                    std::env::var("OPT_LEVEL").unwrap(),
                );
            }
        "#)

        // bar package
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.0.1"))
        .file("bar/src/lib.rs", "")

        // bdep package
        .file("bdep/Cargo.toml", r#"
            [package]
            name = "bdep"
            version = "0.0.1"

            [dependencies]
            bar = { path = "../bar" }
        "#)
        .file("bdep/src/lib.rs", "extern crate bar;")
        .build()
}

#[test]
fn profile_selection_build() {
    let p = all_target_project();

    // Build default targets.
    // NOTES:
    // - bdep `panic` is not set because it thinks `build.rs` is a plugin.
    // - bar `panic` is not set because it is shared with `bdep`.
    // - build_script_build is built without panic because it thinks `build.rs` is a plugin.
    assert_that(p.cargo("build -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..][/]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
"));
    assert_that(
        p.cargo("build -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
",
        ),
    );
}

#[test]
fn profile_selection_build_release() {
    let p = all_target_project();

    // Build default targets, release.
    assert_that(p.cargo("build --release -vv"),
        execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `[..][/]target[/]release[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=release DEBUG=false OPT_LEVEL=3
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[FINISHED] release [optimized] [..]
"));
    assert_that(
        p.cargo("build --release -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] release [optimized] [..]
",
        ),
    );
}

#[test]
fn profile_selection_build_all_targets() {
    let p = all_target_project();
    // Build all explicit targets.
    // NOTES
    // - bdep `panic` is not set because it thinks `build.rs` is a plugin.
    // - bar compiled twice.  It tries with and without panic, but the "is a
    //   plugin" logic is forcing it to be cleared.
    // - build_script_build is built without panic because it thinks
    //   `build.rs` is a plugin.
    // - build_script_build is being run two times.  Once for the `dev` and
    //   `test` targets, once for the `bench` targets.
    //   TODO: "PROFILE" says debug both times, though!
    // - Benchmark dependencies are compiled in `dev` mode, which may be
    //   surprising.  See https://github.com/rust-lang/cargo/issues/4929.
    //
    // - Dependency profiles:
    //   Pkg  Target  Profile     Reason
    //   ---  ------  -------     ------
    //   bar  lib     dev*        For bdep and foo
    //   bar  lib     dev-panic   For tests/benches
    //   bdep lib     dev*        For foo build.rs
    //   foo  custom  dev*
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile    Mode
    //   ------   -------    ----
    //   lib      dev+panic  build  (a normal lib target)
    //   lib      dev-panic  build  (used by tests/benches)
    //   lib      test       test
    //   lib      bench      test(bench)
    //   test     test       test
    //   bench    bench      test(bench)
    //   bin      test       test
    //   bin      bench      test(bench)
    //   bin      dev        build
    //   example  dev        build
    assert_that(p.cargo("build --all-targets -vv"),
        execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..][/]target[/]debug[/]build[/]foo-[..][/]build-script-build`
[RUNNING] `[..][/]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=false OPT_LEVEL=3
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]`
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]`
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]`
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]`
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]`
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]`
[FINISHED] dev [unoptimized + debuginfo] [..]
"));
    assert_that(
        p.cargo("build -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
",
        ),
    );
}

#[test]
fn profile_selection_build_all_targets_release() {
    let p = all_target_project();
    // Build all explicit targets, release.
    // NOTES
    // - bdep `panic` is not set because it thinks `build.rs` is a plugin.
    // - bar compiled twice.  It tries with and without panic, but the "is a
    //   plugin" logic is forcing it to be cleared.
    // - build_script_build is built without panic because it thinks
    //   `build.rs` is a plugin.
    // - build_script_build is being run two times.  Once for the `dev` and
    //   `test` targets, once for the `bench` targets.
    //   TODO: "PROFILE" says debug both times, though!
    //
    // - Dependency profiles:
    //   Pkg  Target  Profile        Reason
    //   ---  ------  -------        ------
    //   bar  lib     release*       For bdep and foo
    //   bar  lib     release-panic  For tests/benches
    //   bdep lib     release*       For foo build.rs
    //   foo  custom  release*
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    //
    // - foo target list is:
    //   Target   Profile        Mode
    //   ------   -------        ----
    //   lib      release+panic  build  (a normal lib target)
    //   lib      release-panic  build  (used by tests/benches)
    //   lib      bench          test   (bench/test de-duped)
    //   test     bench          test
    //   bench    bench          test
    //   bin      bench          test   (bench/test de-duped)
    //   bin      release        build
    //   example  release        build
    assert_that(p.cargo("build --all-targets --release -vv"),
        execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `[..][/]target[/]release[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=release DEBUG=false OPT_LEVEL=3
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]`
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]`
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]`
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]`
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]`
[FINISHED] release [optimized] [..]
"));
    assert_that(
        p.cargo("build --all-targets --release -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] release [optimized] [..]
",
        ),
    );
}

#[test]
fn profile_selection_test() {
    let p = all_target_project();
    // Test default.
    // NOTES:
    // - Dependency profiles:
    //   Pkg  Target  Profile    Reason
    //   ---  ------  -------    ------
    //   bar  lib     dev*       For bdep and foo
    //   bar  lib     dev-panic  For tests/benches
    //   bdep lib     dev*       For foo build.rs
    //   foo  custom  dev*
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile        Mode
    //   ------   -------        ----
    //   lib      dev-panic      build (for tests)
    //   lib      dev            build (for bins)
    //   lib      test           test
    //   test     test           test
    //   example  dev-panic      build
    //   bin      test           test
    //   bin      dev            build
    //
    assert_that(p.cargo("test -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..][/]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C codegen-units=3 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]test1-[..]`
[DOCTEST] foo
[RUNNING] `rustdoc --test [..]
"));
    assert_that(
        p.cargo("test -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]test1-[..]`
[DOCTEST] foo
[RUNNING] `rustdoc --test [..]
",
        ),
    );
}

#[test]
fn profile_selection_test_release() {
    let p = all_target_project();
    // Test default release.
    // NOTES:
    // - Dependency profiles:
    //   Pkg  Target  Profile        Reason
    //   ---  ------  -------        ------
    //   bar  lib     release*       For bdep and foo
    //   bar  lib     release-panic  For tests/benches
    //   bdep lib     release*       For foo build.rs
    //   foo  custom  release*
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile        Mode
    //   ------   -------        ----
    //   lib      release-panic  build  (for tests)
    //   lib      release        build  (for bins)
    //   lib      bench          test
    //   test     bench          test
    //   example  release-panic  build
    //   bin      bench          test
    //   bin      release        build
    //
    assert_that(p.cargo("test --release -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `[..][/]target[/]release[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=release DEBUG=false OPT_LEVEL=3
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[FINISHED] release [optimized] [..]
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]test1-[..]`
[DOCTEST] foo
[RUNNING] `rustdoc --test [..]`
"));
    assert_that(
        p.cargo("test --release -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] release [optimized] [..]
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]foo-[..]`
[RUNNING] `[..][/]deps[/]test1-[..]`
[DOCTEST] foo
[RUNNING] `rustdoc --test [..]
",
        ),
    );
}

#[test]
fn profile_selection_bench() {
    let p = all_target_project();

    // Bench default.
    // NOTES:
    // - Dependency profiles:
    //   Pkg  Target  Profile        Reason
    //   ---  ------  -------        ------
    //   bar  lib     release*       For bdep and foo
    //   bar  lib     release-panic  For tests/benches
    //   bdep lib     release*       For foo build.rs
    //   foo  custom  release*
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile        Mode
    //   ------   -------        ----
    //   lib      release-panic  build (for benches)
    //   lib      release        build (for bins)
    //   lib      bench          test(bench)
    //   bench    bench          test(bench)
    //   bin      bench          test(bench)
    //   bin      release        build
    //
    assert_that(p.cargo("bench -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `[..]target[/]release[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=release DEBUG=false OPT_LEVEL=3
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,link -C opt-level=3 -C codegen-units=4 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[FINISHED] release [optimized] [..]
[RUNNING] `[..][/]deps[/]foo-[..] --bench`
[RUNNING] `[..][/]deps[/]foo-[..] --bench`
[RUNNING] `[..][/]deps[/]bench1-[..] --bench`
"));
    assert_that(
        p.cargo("bench -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] release [optimized] [..]
[RUNNING] `[..][/]deps[/]foo-[..] --bench`
[RUNNING] `[..][/]deps[/]foo-[..] --bench`
[RUNNING] `[..][/]deps[/]bench1-[..] --bench`
",
        ),
    );
}

#[test]
fn profile_selection_check_all_targets() {
    if !is_nightly() {
        // This can be removed once 1.27 is stable, see below.
        return;
    }

    let p = all_target_project();
    // check
    // NOTES:
    // - Dependency profiles:
    //   Pkg  Target  Profile    Action   Reason
    //   ---  ------  -------    ------   ------
    //   bar  lib     dev*       link     For bdep
    //   bar  lib     dev-panic  metadata For tests/benches
    //   bar  lib     dev        metadata For lib/bins
    //   bdep lib     dev*       link     For foo build.rs
    //   foo  custom  dev*       link     For build.rs
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile        Mode
    //   ------   -------        ----
    //   lib      dev            check
    //   lib      dev-panic      check (for tests/benches)
    //   lib      dev-panic      check-test (checking lib as a unittest)
    //   example  dev            check
    //   test     dev-panic      check-test
    //   bench    dev-panic      check-test
    //   bin      dev            check
    //   bin      dev-panic      check-test (checking bin as a unittest)
    //
    assert_that(p.cargo("check --all-targets -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep[..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,metadata -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,metadata -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
"));
    // Starting with Rust 1.27, rustc emits `rmeta` files for bins, so
    // everything should be completely fresh.  Previously, bins were being
    // rechecked.
    // See https://github.com/rust-lang/rust/pull/49289 and
    // https://github.com/rust-lang/cargo/issues/3624
    assert_that(
        p.cargo("check --all-targets -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
",
        ),
    );
}

#[test]
fn profile_selection_check_all_targets_release() {
    if !is_nightly() {
        // See note in profile_selection_check_all_targets.
        return;
    }

    let p = all_target_project();
    // check --release
    // https://github.com/rust-lang/cargo/issues/5218
    // This is a pretty straightforward variant of
    // `profile_selection_check_all_targets` that uses `release` instead of
    // `dev` for all targets.
    assert_that(p.cargo("check --all-targets --release -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[COMPILING] bdep[..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `[..]target[/]release[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=release DEBUG=false OPT_LEVEL=3
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 --test [..]
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 --test [..]
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,metadata -C opt-level=3 -C codegen-units=2 --test [..]
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --crate-type bin --emit=dep-info,metadata -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --crate-type bin --emit=dep-info,metadata -C opt-level=3 -C panic=abort -C codegen-units=2 [..]
[FINISHED] release [optimized] [..]
"));

    assert_that(
        p.cargo("check --all-targets --release -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] release [optimized] [..]
",
        ),
    );
}

#[test]
fn profile_selection_check_all_targets_test() {
    if !is_nightly() {
        // See note in profile_selection_check_all_targets.
        return;
    }

    let p = all_target_project();
    // check --profile=test
    // NOTES:
    // - This doesn't actually use the "test" profile.  Everything uses dev.
    //   It probably should use "test"???  Probably doesn't really matter.
    // - Dependency profiles:
    //   Pkg  Target  Profile    Action   Reason
    //   ---  ------  -------    ------   ------
    //   bar  lib     dev*       link     For bdep
    //   bar  lib     dev-panic  metadata For tests/benches
    //   bdep lib     dev*       link     For foo build.rs
    //   foo  custom  dev*       link     For build.rs
    //
    //   `*` = wants panic, but it is cleared when args are built.
    //
    // - foo target list is:
    //   Target   Profile    Mode
    //   ------   -------    ----
    //   lib      dev-panic  check-test (for tests/benches)
    //   lib      dev-panic  check-test (checking lib as a unittest)
    //   example  dev-panic  check-test
    //   test     dev-panic  check-test
    //   bench    dev-panic  check-test
    //   bin      dev-panic  check-test
    //
    assert_that(p.cargo("check --all-targets --profile=test -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep[..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[RUNNING] `rustc --crate-name foo src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustc --crate-name foo src[/]lib.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name test1 tests[/]test1.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name foo src[/]main.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name bench1 benches[/]bench1.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[RUNNING] `rustc --crate-name ex1 examples[/]ex1.rs --emit=dep-info,metadata -C codegen-units=1 -C debuginfo=2 --test [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
"));

    assert_that(
        p.cargo("check --all-targets --profile=test -vv"),
        execs().with_status(0).with_stderr_unordered(
            "\
[FRESH] bar [..]
[FRESH] bdep [..]
[FRESH] foo [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
",
        ),
    );
}

#[test]
fn profile_selection_doc() {
    let p = all_target_project();
    // doc
    // NOTES:
    // - Dependency profiles:
    //   Pkg  Target  Profile    Action   Reason
    //   ---  ------  -------    ------   ------
    //   bar  lib     dev*       link     For bdep
    //   bar  lib     dev        metadata For rustdoc
    //   bdep lib     dev*       link     For foo build.rs
    //   foo  custom  dev*       link     For build.rs
    //
    //   `*` = wants panic, but it is cleared when args are built.
    assert_that(p.cargo("doc -vv"), execs().with_status(0).with_stderr_unordered("\
[COMPILING] bar [..]
[DOCUMENTING] bar [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `rustdoc --crate-name bar bar[/]src[/]lib.rs [..]
[RUNNING] `rustc --crate-name bar bar[/]src[/]lib.rs --crate-type lib --emit=dep-info,metadata -C panic=abort -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] bdep [..]
[RUNNING] `rustc --crate-name bdep bdep[/]src[/]lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name build_script_build build.rs --crate-type bin --emit=dep-info,link -C codegen-units=1 -C debuginfo=2 [..]
[RUNNING] `[..]target[/]debug[/]build[/]foo-[..][/]build-script-build`
foo custom build PROFILE=debug DEBUG=true OPT_LEVEL=0
[DOCUMENTING] foo [..]
[RUNNING] `rustdoc --crate-name foo src[/]lib.rs [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
"));
}
