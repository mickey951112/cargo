use std::fs::{self, File};
use std::io::prelude::*;

use support::registry::Package;
use support::{basic_manifest, paths, project, ProjectBuilder};

#[test]
fn adding_and_removing_packages() {
    let p = project()
        .file("src/main.rs", "fn main() {}")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.0.1"))
        .file("bar/src/lib.rs", "")
        .build();

    p.cargo("generate-lockfile").run();

    let toml = p.root().join("Cargo.toml");
    let lock1 = p.read_lockfile();

    // add a dep
    File::create(&toml)
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        authors = []
        version = "0.0.1"

        [dependencies.bar]
        path = "bar"
    "#,
        ).unwrap();
    p.cargo("generate-lockfile").run();
    let lock2 = p.read_lockfile();
    assert_ne!(lock1, lock2);

    // change the dep
    File::create(&p.root().join("bar/Cargo.toml"))
        .unwrap()
        .write_all(basic_manifest("bar", "0.0.2").as_bytes())
        .unwrap();
    p.cargo("generate-lockfile").run();
    let lock3 = p.read_lockfile();
    assert_ne!(lock1, lock3);
    assert_ne!(lock2, lock3);

    // remove the dep
    println!("lock4");
    File::create(&toml)
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        authors = []
        version = "0.0.1"
    "#,
        ).unwrap();
    p.cargo("generate-lockfile").run();
    let lock4 = p.read_lockfile();
    assert_eq!(lock1, lock4);
}

#[test]
fn no_index_update() {
    Package::new("serde", "1.0.0").publish();

    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            authors = []
            version = "0.0.1"

            [dependencies]
            serde = "1.0"
        "#,
        ).file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("generate-lockfile")
        .with_stderr("[UPDATING] `[..]` index")
        .run();

    p.cargo("generate-lockfile -Zno-index-update")
        .masquerade_as_nightly_cargo()
        .with_stdout("")
        .with_stderr("")
        .run();
}

#[test]
fn preserve_metadata() {
    let p = project()
        .file("src/main.rs", "fn main() {}")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.0.1"))
        .file("bar/src/lib.rs", "")
        .build();

    p.cargo("generate-lockfile").run();

    let metadata = r#"
[metadata]
bar = "baz"
foo = "bar"
"#;
    let lockfile = p.root().join("Cargo.lock");
    let lock = p.read_lockfile();
    let data = lock + metadata;
    File::create(&lockfile)
        .unwrap()
        .write_all(data.as_bytes())
        .unwrap();

    // Build and make sure the metadata is still there
    p.cargo("build").run();
    let lock = p.read_lockfile();
    assert!(lock.contains(metadata.trim()), "{}", lock);

    // Update and make sure the metadata is still there
    p.cargo("update").run();
    let lock = p.read_lockfile();
    assert!(lock.contains(metadata.trim()), "{}", lock);
}

#[test]
fn preserve_line_endings_issue_2076() {
    let p = project()
        .file("src/main.rs", "fn main() {}")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.0.1"))
        .file("bar/src/lib.rs", "")
        .build();

    let lockfile = p.root().join("Cargo.lock");
    p.cargo("generate-lockfile").run();
    assert!(lockfile.is_file());
    p.cargo("generate-lockfile").run();

    let lock0 = p.read_lockfile();

    assert!(lock0.starts_with("[[package]]\n"));

    let lock1 = lock0.replace("\n", "\r\n");
    {
        File::create(&lockfile)
            .unwrap()
            .write_all(lock1.as_bytes())
            .unwrap();
    }

    p.cargo("generate-lockfile").run();

    let lock2 = p.read_lockfile();

    assert!(lock2.starts_with("[[package]]\r\n"));
    assert_eq!(lock1, lock2);
}

#[test]
fn cargo_update_generate_lockfile() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    let lockfile = p.root().join("Cargo.lock");
    assert!(!lockfile.is_file());
    p.cargo("update").with_stdout("").run();
    assert!(lockfile.is_file());

    fs::remove_file(p.root().join("Cargo.lock")).unwrap();

    assert!(!lockfile.is_file());
    p.cargo("update").with_stdout("").run();
    assert!(lockfile.is_file());
}

#[test]
fn duplicate_entries_in_lockfile() {
    let _a = ProjectBuilder::new(paths::root().join("a"))
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "a"
            authors = []
            version = "0.0.1"

            [dependencies]
            common = {path="common"}
            "#,
        ).file("src/lib.rs", "")
        .build();

    let common_toml = &basic_manifest("common", "0.0.1");

    let _common_in_a = ProjectBuilder::new(paths::root().join("a/common"))
        .file("Cargo.toml", common_toml)
        .file("src/lib.rs", "")
        .build();

    let b = ProjectBuilder::new(paths::root().join("b"))
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "b"
            authors = []
            version = "0.0.1"

            [dependencies]
            common = {path="common"}
            a = {path="../a"}
            "#,
        ).file("src/lib.rs", "")
        .build();

    let _common_in_b = ProjectBuilder::new(paths::root().join("b/common"))
        .file("Cargo.toml", common_toml)
        .file("src/lib.rs", "")
        .build();

    // should fail due to a duplicate package `common` in the lockfile
    b.cargo("build")
        .with_status(101)
        .with_stderr_contains(
            "[..]package collision in the lockfile: packages common [..] and \
             common [..] are different, but only one can be written to \
             lockfile unambigiously",
        ).run();
}
