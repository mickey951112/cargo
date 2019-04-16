use std;
use std::fs::File;

use crate::support::registry::Package;
use crate::support::{
    basic_manifest, cargo_process, git, paths, project, publish::validate_crate_contents,
};

fn pl_manifest(name: &str, version: &str, extra: &str) -> String {
    format!(
        r#"
        cargo-features = ["publish-lockfile"]

        [project]
        name = "{}"
        version = "{}"
        authors = []
        license = "MIT"
        description = "foo"
        documentation = "foo"
        homepage = "foo"
        repository = "foo"
        publish-lockfile = true

        {}
        "#,
        name, version, extra
    )
}

#[test]
fn package_lockfile() {
    let p = project()
        .file("Cargo.toml", &pl_manifest("foo", "0.0.1", ""))
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("package")
        .masquerade_as_nightly_cargo()
        .with_stderr(
            "\
[PACKAGING] foo v0.0.1 ([CWD])
[VERIFYING] foo v0.0.1 ([CWD])
[COMPILING] foo v0.0.1 ([CWD][..])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        )
        .run();
    assert!(p.root().join("target/package/foo-0.0.1.crate").is_file());
    p.cargo("package -l")
        .masquerade_as_nightly_cargo()
        .with_stdout(
            "\
Cargo.lock
Cargo.toml
src/main.rs
",
        )
        .run();
    p.cargo("package")
        .masquerade_as_nightly_cargo()
        .with_stdout("")
        .run();

    let f = File::open(&p.root().join("target/package/foo-0.0.1.crate")).unwrap();
    validate_crate_contents(
        f,
        "foo-0.0.1.crate",
        &["Cargo.toml", "Cargo.toml.orig", "Cargo.lock", "src/main.rs"],
        &[],
    );
}

#[test]
fn package_lockfile_git_repo() {
    // Create a Git repository containing a minimal Rust project.
    let g = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &pl_manifest("foo", "0.0.1", ""))
        .file("src/main.rs", "fn main() {}")
        .build();
    cargo_process("package -l")
        .cwd(g.root())
        .masquerade_as_nightly_cargo()
        .with_stdout(
            "\
.cargo_vcs_info.json
Cargo.lock
Cargo.toml
src/main.rs
",
        )
        .run();
}

#[test]
fn no_lock_file_with_library() {
    let p = project()
        .file("Cargo.toml", &pl_manifest("foo", "0.0.1", ""))
        .file("src/lib.rs", "")
        .build();

    p.cargo("package").masquerade_as_nightly_cargo().run();

    let f = File::open(&p.root().join("target/package/foo-0.0.1.crate")).unwrap();
    validate_crate_contents(
        f,
        "foo-0.0.1.crate",
        &["Cargo.toml", "Cargo.toml.orig", "src/lib.rs"],
        &[],
    );
}

#[test]
fn lock_file_and_workspace() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [workspace]
            members = ["foo"]
        "#,
        )
        .file("foo/Cargo.toml", &pl_manifest("foo", "0.0.1", ""))
        .file("foo/src/main.rs", "fn main() {}")
        .build();

    p.cargo("package")
        .cwd("foo")
        .masquerade_as_nightly_cargo()
        .run();

    let f = File::open(&p.root().join("target/package/foo-0.0.1.crate")).unwrap();
    validate_crate_contents(
        f,
        "foo-0.0.1.crate",
        &["Cargo.toml", "Cargo.toml.orig", "src/main.rs", "Cargo.lock"],
        &[],
    );
}

#[test]
fn warn_resolve_changes() {
    // `multi` has multiple sources (path and registry).
    Package::new("mutli", "0.1.0").publish();
    // `updated` is always from registry, but should not change.
    Package::new("updated", "1.0.0").publish();
    // `patched` is [patch]ed.
    Package::new("patched", "1.0.0").publish();

    let p = project()
        .file(
            "Cargo.toml",
            &pl_manifest(
                "foo",
                "0.0.1",
                r#"
                [dependencies]
                mutli = { path = "mutli", version = "0.1" }
                updated = "1.0"
                patched = "1.0"

                [patch.crates-io]
                patched = { path = "patched" }
                "#,
            ),
        )
        .file("src/main.rs", "fn main() {}")
        .file("mutli/Cargo.toml", &basic_manifest("mutli", "0.1.0"))
        .file("mutli/src/lib.rs", "")
        .file("patched/Cargo.toml", &basic_manifest("patched", "1.0.0"))
        .file("patched/src/lib.rs", "")
        .build();

    p.cargo("generate-lockfile")
        .masquerade_as_nightly_cargo()
        .run();

    // Make sure this does not change or warn.
    Package::new("updated", "1.0.1").publish();

    p.cargo("package --no-verify")
        .masquerade_as_nightly_cargo()
        .with_stderr_unordered(
            "\
[PACKAGING] foo v0.0.1 ([..])
[UPDATING] `[..]` index
[WARNING] package `mutli v0.1.0` added to Cargo.lock, was originally sourced from `[..]/foo/mutli`
[WARNING] package `patched v1.0.0` added to Cargo.lock, was originally sourced from `[..]/foo/patched`
",
        )
        .run();
}

#[test]
fn outdated_lock_version_change_does_not_warn() {
    // If the version of the package being packaged changes, but Cargo.lock is
    // not updated, don't bother warning about it.
    let p = project()
        .file("Cargo.toml", &pl_manifest("foo", "0.1.0", ""))
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("generate-lockfile")
        .masquerade_as_nightly_cargo()
        .run();

    p.change_file("Cargo.toml", &pl_manifest("foo", "0.2.0", ""));

    p.cargo("package --no-verify")
        .masquerade_as_nightly_cargo()
        .with_stderr("[PACKAGING] foo v0.2.0 ([..])")
        .run();
}

#[test]
fn no_warn_workspace_extras() {
    // Other entries in workspace lock file should be ignored.
    Package::new("dep1", "1.0.0").publish();
    Package::new("dep2", "1.0.0").publish();

    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [workspace]
            members = ["a", "b"]
            "#,
        )
        .file(
            "a/Cargo.toml",
            &pl_manifest(
                "a",
                "0.1.0",
                r#"
                [dependencies]
                dep1 = "1.0"
                "#,
            ),
        )
        .file("a/src/main.rs", "fn main() {}")
        .file(
            "b/Cargo.toml",
            &pl_manifest(
                "b",
                "0.1.0",
                r#"
                [dependencies]
                dep2 = "1.0"
                "#,
            ),
        )
        .file("b/src/main.rs", "fn main() {}")
        .build();
    p.cargo("generate-lockfile")
        .masquerade_as_nightly_cargo()
        .run();
    p.cargo("package --no-verify")
        .cwd("a")
        .masquerade_as_nightly_cargo()
        .with_stderr(
            "\
[PACKAGING] a v0.1.0 ([..])
[UPDATING] `[..]` index
",
        )
        .run();
}

#[test]
fn out_of_date_lock_warn() {
    // Dependency is force-changed from an out-of-date Cargo.lock.
    Package::new("dep", "1.0.0").publish();
    Package::new("dep", "2.0.0").publish();

    let p = project()
        .file(
            "Cargo.toml",
            &pl_manifest(
                "foo",
                "0.0.1",
                r#"
                [dependencies]
                dep = "1.0"
                "#,
            ),
        )
        .file("src/main.rs", "fn main() {}")
        .build();
    p.cargo("generate-lockfile")
        .masquerade_as_nightly_cargo()
        .run();
    p.change_file(
        "Cargo.toml",
        &pl_manifest(
            "foo",
            "0.0.1",
            r#"
            [dependencies]
            dep = "2.0"
            "#,
        ),
    );
    p.cargo("package --no-verify")
        .masquerade_as_nightly_cargo()
        .with_stderr(
            "\
[PACKAGING] foo v0.0.1 ([..])
[UPDATING] `[..]` index
[WARNING] package `dep v2.0.0` added to Cargo.lock, previous version was `1.0.0`
",
        )
        .run();
}

#[test]
fn warn_package_with_yanked() {
    Package::new("bar", "0.1.0").publish();
    let p = project()
        .file(
            "Cargo.toml",
            &pl_manifest(
                "foo",
                "0.0.1",
                r#"
                [dependencies]
                bar = "0.1"
                "#,
            ),
        )
        .file("src/main.rs", "fn main() {}")
        .build();
    p.cargo("generate-lockfile")
        .masquerade_as_nightly_cargo()
        .run();
    Package::new("bar", "0.1.0").yanked(true).publish();
    // Make sure it sticks with the locked (yanked) version.
    Package::new("bar", "0.1.1").publish();
    p.cargo("package --no-verify")
        .masquerade_as_nightly_cargo()
        .with_stderr(
            "\
[PACKAGING] foo v0.0.1 ([..])
[UPDATING] `[..]` index
[WARNING] package `bar v0.1.0` in Cargo.lock is yanked in registry \
    `crates.io`, consider updating to a version that is not yanked
",
        )
        .run();
}

#[test]
fn warn_install_with_yanked() {
    Package::new("bar", "0.1.0").yanked(true).publish();
    Package::new("bar", "0.1.1").publish();
    Package::new("foo", "0.1.0")
        .dep("bar", "0.1")
        .file("src/main.rs", "fn main() {}")
        .file(
            "Cargo.lock",
            r#"
[[package]]
name = "bar"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "bar 0.1.0 (registry+https://github.com/rust-lang/crates.io-index)",
]
            "#,
        )
        .publish();

    // It is unfortunate that this displays UPDATING twice.
    cargo_process("install --locked foo")
        .with_stderr(
            "\
[UPDATING] `[..]` index
[DOWNLOADING] crates ...
[DOWNLOADED] foo v0.1.0 (registry `[..]`)
[INSTALLING] foo v0.1.0
[UPDATING] `[..]` index
[WARNING] package `bar v0.1.0` in Cargo.lock is yanked in registry \
    `crates.io`, consider running without --locked
[DOWNLOADING] crates ...
[DOWNLOADED] bar v0.1.0 (registry `[..]`)
[COMPILING] bar v0.1.0
[COMPILING] foo v0.1.0
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] [..]/.cargo/bin/foo[EXE]
[INSTALLED] package `foo v0.1.0` (executable `foo[EXE]`)
[WARNING] be sure to add [..]
",
        )
        .run();

    // Try again without --locked, make sure it uses 0.1.1 and does not warn.
    cargo_process("install --force foo")
        .with_stderr(
            "\
[UPDATING] `[..]` index
[INSTALLING] foo v0.1.0
[DOWNLOADING] crates ...
[DOWNLOADED] bar v0.1.1 (registry `[..]`)
[COMPILING] bar v0.1.1
[COMPILING] foo v0.1.0
[FINISHED] release [optimized] target(s) in [..]
[REPLACING] [..]/.cargo/bin/foo[EXE]
[REPLACED] package `foo v0.1.0` with `foo v0.1.0` (executable `foo[EXE]`)
[WARNING] be sure to add [..]
",
        )
        .run();
}

#[test]
fn publish_lockfile_default() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["publish-lockfile"]
            [package]
            name = "foo"
            version = "1.0.0"
            authors = []
            license = "MIT"
            description = "foo"
            documentation = "foo"
            homepage = "foo"
            repository = "foo"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("package -l")
        .masquerade_as_nightly_cargo()
        .with_stdout(
            "\
Cargo.lock
Cargo.toml
src/main.rs
",
        )
        .run();

    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["publish-lockfile"]
            [package]
            name = "foo"
            version = "1.0.0"
            authors = []
            license = "MIT"
            description = "foo"
            documentation = "foo"
            homepage = "foo"
            repository = "foo"
            publish-lockfile = false
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("package -l")
        .masquerade_as_nightly_cargo()
        .with_stdout(
            "\
Cargo.toml
src/main.rs
",
        )
        .run();
}
