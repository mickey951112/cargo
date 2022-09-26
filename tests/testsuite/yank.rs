//! Tests for the `cargo yank` command.

use std::fs;

use cargo_test_support::paths::CargoPathExt;
use cargo_test_support::project;
use cargo_test_support::registry;

fn setup(name: &str, version: &str) {
    let dir = registry::api_path().join(format!("api/v1/crates/{}/{}", name, version));
    dir.mkdir_p();
    fs::write(dir.join("yank"), r#"{"ok": true}"#).unwrap();
}

#[cargo_test]
fn explicit_version() {
    registry::init();
    setup("foo", "0.0.1");

    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                license = "MIT"
                description = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("yank --version 0.0.1 --token sekrit").run();

    p.cargo("yank --undo --version 0.0.1 --token sekrit")
        .with_status(101)
        .with_stderr(
            "    Updating `[..]` index
      Unyank foo@0.0.1
error: failed to undo a yank from the registry at file:///[..]

Caused by:
  EOF while parsing a value at line 1 column 0",
        )
        .run();
}

#[cargo_test]
fn inline_version() {
    registry::init();
    setup("foo", "0.0.1");

    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                license = "MIT"
                description = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("yank foo@0.0.1 --token sekrit").run();

    p.cargo("yank --undo foo@0.0.1 --token sekrit")
        .with_status(101)
        .with_stderr(
            "    Updating `[..]` index
      Unyank foo@0.0.1
error: failed to undo a yank from the registry at file:///[..]

Caused by:
  EOF while parsing a value at line 1 column 0",
        )
        .run();
}

#[cargo_test]
fn version_required() {
    registry::init();
    setup("foo", "0.0.1");

    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                license = "MIT"
                description = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("yank foo --token sekrit")
        .with_status(101)
        .with_stderr("error: `--version` is required")
        .run();
}

#[cargo_test]
fn inline_version_without_name() {
    registry::init();
    setup("foo", "0.0.1");

    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                license = "MIT"
                description = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("yank @0.0.1 --token sekrit")
        .with_status(101)
        .with_stderr("error: missing crate name for `@0.0.1`")
        .run();
}

#[cargo_test]
fn inline_and_explicit_version() {
    registry::init();
    setup("foo", "0.0.1");

    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                license = "MIT"
                description = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("yank foo@0.0.1 --version 0.0.1 --token sekrit")
        .with_status(101)
        .with_stderr("error: cannot specify both `@0.0.1` and `--version`")
        .run();
}
