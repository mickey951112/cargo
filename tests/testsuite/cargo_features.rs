use support::ChannelChanger;
use support::{execs, project, publish};
use support::hamcrest::assert_that;

#[test]
fn feature_required() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            im-a-teapot = true
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
  the `im-a-teapot` manifest key is unstable and may not work properly in England

Caused by:
  feature `test-dummy-unstable` is required

consider adding `cargo-features = [\"test-dummy-unstable\"]` to the manifest
",
        ),
    );

    assert_that(
        p.cargo("build"),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  the `im-a-teapot` manifest key is unstable and may not work properly in England

Caused by:
  feature `test-dummy-unstable` is required

this Cargo does not support nightly features, but if you
switch to nightly channel you can add
`cargo-features = [\"test-dummy-unstable\"]` to enable this feature
",
        ),
    );
}

#[test]
fn unknown_feature() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["foo"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build"),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  unknown cargo feature `foo`
",
        ),
    );
}

#[test]
fn stable_feature_warns() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["test-dummy-stable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build"),
        execs().with_stderr(
            "\
warning: the cargo feature `test-dummy-stable` is now stable and is no longer \
necessary to be listed in the manifest
[COMPILING] a [..]
[FINISHED] [..]
",
        ),
    );
}

#[test]
fn nightly_feature_requires_nightly() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["test-dummy-unstable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            im-a-teapot = true
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_stderr(
            "\
[COMPILING] a [..]
[FINISHED] [..]
",
        ),
    );

    assert_that(
        p.cargo("build"),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  the cargo feature `test-dummy-unstable` requires a nightly version of Cargo, \
  but this is the `stable` channel
",
        ),
    );
}

#[test]
fn nightly_feature_requires_nightly_in_dep() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a" }
        "#,
        )
        .file("src/lib.rs", "")
        .file(
            "a/Cargo.toml",
            r#"
            cargo-features = ["test-dummy-unstable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            im-a-teapot = true
        "#,
        )
        .file("a/src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_stderr(
            "\
[COMPILING] a [..]
[COMPILING] b [..]
[FINISHED] [..]
",
        ),
    );

    assert_that(
        p.cargo("build"),
        execs().with_status(101).with_stderr(
            "\
error: failed to load source for a dependency on `a`

Caused by:
  Unable to update [..]

Caused by:
  failed to parse manifest at `[..]`

Caused by:
  the cargo feature `test-dummy-unstable` requires a nightly version of Cargo, \
  but this is the `stable` channel
",
        ),
    );
}

#[test]
fn cant_publish() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["test-dummy-unstable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            im-a-teapot = true
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build").masquerade_as_nightly_cargo(),
        execs().with_stderr(
            "\
[COMPILING] a [..]
[FINISHED] [..]
",
        ),
    );

    assert_that(
        p.cargo("build"),
        execs().with_status(101).with_stderr(
            "\
error: failed to parse manifest at `[..]`

Caused by:
  the cargo feature `test-dummy-unstable` requires a nightly version of Cargo, \
  but this is the `stable` channel
",
        ),
    );
}

#[test]
fn z_flags_rejected() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["test-dummy-unstable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            im-a-teapot = true
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("build -Zprint-im-a-teapot"),
        execs()
            .with_status(101)
            .with_stderr("error: the `-Z` flag is only accepted on the nightly channel of Cargo"),
    );

    assert_that(
        p.cargo("build -Zarg").masquerade_as_nightly_cargo(),
        execs()
            .with_status(101)
            .with_stderr("error: unknown `-Z` flag specified: arg"),
    );

    assert_that(
        p.cargo("build -Zprint-im-a-teapot")
            .masquerade_as_nightly_cargo(),
        execs()
            .with_stdout("im-a-teapot = true\n")
            .with_stderr(
                "\
[COMPILING] a [..]
[FINISHED] [..]
",
            ),
    );
}

#[test]
fn publish_allowed() {
    publish::setup();

    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["test-dummy-unstable"]

            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("src/lib.rs", "")
        .build();
    assert_that(
        p.cargo("publish --index")
            .arg(publish::registry().to_string())
            .masquerade_as_nightly_cargo(),
        execs(),
    );
}
