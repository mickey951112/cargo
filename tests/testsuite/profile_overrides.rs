use support::{basic_manifest, basic_lib_manifest, execs, project};
use support::ChannelChanger;
use support::hamcrest::assert_that;

#[test]
fn profile_override_gated() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [profile.dev.build-override]
            opt-level = 3
        "#,
        )
        .file("src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  feature `profile-overrides` is required

consider adding `cargo-features = [\"profile-overrides\"]` to the manifest
",
        ),
    );

    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [profile.dev.overrides."*"]
            opt-level = 3
        "#,
        )
        .file("src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  feature `profile-overrides` is required

consider adding `cargo-features = [\"profile-overrides\"]` to the manifest
",
        ),
    );
}

#[test]
fn profile_override_basic() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = {path = "bar"}

            [profile.dev]
            opt-level = 1

            [profile.dev.overrides.bar]
            opt-level = 3
        "#,
        )
        .file("src/lib.rs", "")
        .file("bar/Cargo.toml", &basic_lib_manifest("bar"))
        .file("bar/src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build -v").masquerade_as_nightly_cargo(),
        execs().with_stderr(
            "[COMPILING] bar [..]
[RUNNING] `rustc --crate-name bar [..] -C opt-level=3 [..]`
[COMPILING] foo [..]
[RUNNING] `rustc --crate-name foo [..] -C opt-level=1 [..]`
[FINISHED] dev [optimized + debuginfo] target(s) in [..]",
        ),
    );
}

#[test]
fn profile_override_warnings() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [package]
            name = "foo"
            version = "0.0.1"

            [dependencies]
            bar = {path = "bar"}

            [profile.dev.overrides.bart]
            opt-level = 3

            [profile.dev.overrides.no-suggestion]
            opt-level = 3

            [profile.dev.overrides."bar:1.2.3"]
            opt-level = 3
        "#,
        )
        .file("src/lib.rs", "")
        .file("bar/Cargo.toml", &basic_lib_manifest("bar"))
        .file("bar/src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_stderr_contains(
            "\
[WARNING] version or URL in profile override spec `bar:1.2.3` does not match any of the packages: bar v0.5.0 ([..])
[WARNING] profile override spec `bart` did not match any packages

Did you mean `bar`?
[WARNING] profile override spec `no-suggestion` did not match any packages
[COMPILING] [..]
",
        ),
    );
}

#[test]
fn profile_override_dev_release_only() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [package]
            name = "foo"
            version = "0.0.1"

            [dependencies]
            bar = {path = "bar"}

            [profile.test.overrides.bar]
            opt-level = 3
        "#,
        )
        .file("src/lib.rs", "")
        .file("bar/Cargo.toml", &basic_lib_manifest("bar"))
        .file("bar/src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_status(101).with_stderr_contains(
            "\
Caused by:
  Profile overrides may only be specified for `dev` or `release` profile, not `test`.
",
        ),
    );
}

#[test]
fn profile_override_bad_settings() {
    let bad_values = [
        (
            "panic = \"abort\"",
            "`panic` may not be specified in a profile override.",
        ),
        (
            "lto = true",
            "`lto` may not be specified in a profile override.",
        ),
        (
            "rpath = true",
            "`rpath` may not be specified in a profile override.",
        ),
        ("overrides = {}", "Profile overrides cannot be nested."),
    ];
    for &(ref snippet, ref expected) in bad_values.iter() {
        let p = project()
            .file(
                "Cargo.toml",
                &format!(
                    r#"
                cargo-features = ["profile-overrides"]

                [package]
                name = "foo"
                version = "0.0.1"

                [dependencies]
                bar = {{path = "bar"}}

                [profile.dev.overrides.bar]
                {}
            "#,
                    snippet
                ),
            )
            .file("src/lib.rs", "")
            .file("bar/Cargo.toml", &basic_lib_manifest("bar"))
            .file("bar/src/lib.rs", "")
            .build();

        assert_that(
            p.cargo("build").masquerade_as_nightly_cargo(),
            execs()
                .with_status(101)
                .with_stderr_contains(format!("Caused by:\n  {}", expected)),
        );
    }
}

#[test]
fn profile_override_hierarchy() {
    // Test that the precedence rules are correct for different types.
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [workspace]
            members = ["m1", "m2", "m3"]

            [profile.dev]
            codegen-units = 1

            [profile.dev.overrides.m2]
            codegen-units = 2

            [profile.dev.overrides."*"]
            codegen-units = 3

            [profile.dev.build-override]
            codegen-units = 4
            "#)

        // m1
        .file("m1/Cargo.toml",
            r#"
            [package]
            name = "m1"
            version = "0.0.1"

            [dependencies]
            m2 = { path = "../m2" }
            dep = { path = "../../dep" }
            "#)
        .file("m1/src/lib.rs", "extern crate m2; extern crate dep;")
        .file("m1/build.rs", "fn main() {}")

        // m2
        .file("m2/Cargo.toml",
            r#"
            [package]
            name = "m2"
            version = "0.0.1"

            [dependencies]
            m3 = { path = "../m3" }

            [build-dependencies]
            m3 = { path = "../m3" }
            dep = { path = "../../dep" }
            "#)
        .file("m2/src/lib.rs", "extern crate m3;")
        .file("m2/build.rs", "extern crate m3; extern crate dep; fn main() {}")

        // m3
        .file("m3/Cargo.toml", &basic_lib_manifest("m3"))
        .file("m3/src/lib.rs", "")
        .build();

    // dep (outside of workspace)
    let _dep = project().at("dep")
        .file("Cargo.toml", &basic_lib_manifest("dep"))
        .file("src/lib.rs", "")
        .build();

    // Profiles should be:
    // m3: 4 (as build.rs dependency)
    // m3: 1 (as [profile.dev] as workspace member)
    // dep: 3 (as [profile.dev.overrides."*"] as non-workspace member)
    // m1 build.rs: 4 (as [profile.dev.build-override])
    // m2 build.rs: 2 (as [profile.dev.overrides.m2])
    // m2: 2 (as [profile.dev.overrides.m2])
    // m1: 1 (as [profile.dev])

    assert_that(
        p.cargo("build -v").masquerade_as_nightly_cargo(),
        execs().with_stderr_unordered("\
[COMPILING] m3 [..]
[COMPILING] dep [..]
[RUNNING] `rustc --crate-name m3 m3/src/lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=4 [..]
[RUNNING] `rustc --crate-name dep [..]dep/src/lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=3 [..]
[RUNNING] `rustc --crate-name m3 m3/src/lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 [..]
[RUNNING] `rustc --crate-name build_script_build m1/build.rs --crate-type bin --emit=dep-info,link -C codegen-units=4 [..]
[COMPILING] m2 [..]
[RUNNING] `rustc --crate-name build_script_build m2/build.rs --crate-type bin --emit=dep-info,link -C codegen-units=2 [..]
[RUNNING] `[..]/m1-[..]/build-script-build`
[RUNNING] `[..]/m2-[..]/build-script-build`
[RUNNING] `rustc --crate-name m2 m2/src/lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=2 [..]
[COMPILING] m1 [..]
[RUNNING] `rustc --crate-name m1 m1/src/lib.rs --crate-type lib --emit=dep-info,link -C codegen-units=1 [..]
[FINISHED] dev [unoptimized + debuginfo] [..]
",
        ),
    );
}

#[test]
fn profile_override_spec_multiple() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [package]
            name = "foo"
            version = "0.0.1"

            [dependencies]
            bar = { path = "bar" }

            [profile.dev.overrides.bar]
            opt-level = 3

            [profile.dev.overrides."bar:0.5.0"]
            opt-level = 3
            "#,
        )
        .file("src/lib.rs", "")
        .file("bar/Cargo.toml", &basic_lib_manifest("bar"))
        .file("bar/src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build -v").masquerade_as_nightly_cargo(),
        execs().with_status(101).with_stderr_contains(
            "\
[ERROR] multiple profile overrides in profile `dev` match package `bar v0.5.0 ([..])`
found profile override specs: bar, bar:0.5.0",
        ),
    );
}

#[test]
fn profile_override_spec() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["profile-overrides"]

            [workspace]
            members = ["m1", "m2"]

            [profile.dev.overrides."dep:1.0.0"]
            codegen-units = 1

            [profile.dev.overrides."dep:2.0.0"]
            codegen-units = 2
            "#)

        // m1
        .file("m1/Cargo.toml",
            r#"
            [package]
            name = "m1"
            version = "0.0.1"

            [dependencies]
            dep = { path = "../../dep1" }
            "#)
        .file("m1/src/lib.rs", "extern crate dep;")

        // m2
        .file("m2/Cargo.toml",
            r#"
            [package]
            name = "m2"
            version = "0.0.1"

            [dependencies]
            dep = {path = "../../dep2" }
            "#)
        .file("m2/src/lib.rs", "extern crate dep;")

        .build();

    project().at("dep1")
        .file("Cargo.toml", &basic_manifest("dep", "1.0.0"))
        .file("src/lib.rs", "")
        .build();

    project().at("dep2")
        .file("Cargo.toml", &basic_manifest("dep", "2.0.0"))
        .file("src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build -v").masquerade_as_nightly_cargo(),
        execs()
            .with_stderr_contains(
                "[RUNNING] `rustc [..]dep1/src/lib.rs [..] -C codegen-units=1 [..]",
            )
            .with_stderr_contains(
                "[RUNNING] `rustc [..]dep2/src/lib.rs [..] -C codegen-units=2 [..]",
            ),
    );
}
