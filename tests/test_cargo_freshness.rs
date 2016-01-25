use std::fs::{self, File};
use std::io::prelude::*;

use support::{project, execs, path2url};
use support::COMPILING;
use support::paths::CargoPathExt;
use hamcrest::{assert_that, existing_file};

fn setup() {}

test!(modifying_and_moving {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            authors = []
            version = "0.0.1"
        "#)
        .file("src/main.rs", r#"
            mod a; fn main() {}
        "#)
        .file("src/a.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0).with_stdout(format!("\
{compiling} foo v0.0.1 ({dir})
", compiling = COMPILING, dir = path2url(p.root()))));

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));
    p.root().move_into_the_past().unwrap();
    p.root().join("target").move_into_the_past().unwrap();

    File::create(&p.root().join("src/a.rs")).unwrap()
         .write_all(b"fn main() {}").unwrap();
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(format!("\
{compiling} foo v0.0.1 ({dir})
", compiling = COMPILING, dir = path2url(p.root()))));

    fs::rename(&p.root().join("src/a.rs"), &p.root().join("src/b.rs")).unwrap();
    assert_that(p.cargo("build"),
                execs().with_status(101));
});

test!(modify_only_some_files {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            authors = []
            version = "0.0.1"
        "#)
        .file("src/lib.rs", "mod a;")
        .file("src/a.rs", "")
        .file("src/main.rs", r#"
            mod b;
            fn main() {}
        "#)
        .file("src/b.rs", "")
        .file("tests/test.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0).with_stdout(format!("\
{compiling} foo v0.0.1 ({dir})
", compiling = COMPILING, dir = path2url(p.root()))));
    assert_that(p.cargo("test"),
                execs().with_status(0));
    ::sleep_ms(1000);

    assert_that(&p.bin("foo"), existing_file());

    let lib = p.root().join("src/lib.rs");
    let bin = p.root().join("src/b.rs");

    File::create(&lib).unwrap().write_all(b"invalid rust code").unwrap();
    File::create(&bin).unwrap().write_all(b"fn foo() {}").unwrap();
    lib.move_into_the_past().unwrap();

    // Make sure the binary is rebuilt, not the lib
    assert_that(p.cargo("build")
                 .env("RUST_LOG", "cargo::ops::cargo_rustc::fingerprint"),
                execs().with_status(0).with_stdout(format!("\
{compiling} foo v0.0.1 ({dir})
", compiling = COMPILING, dir = path2url(p.root()))));
    assert_that(&p.bin("foo"), existing_file());
});

test!(rebuild_sub_package_then_while_package {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            authors = []
            version = "0.0.1"

            [dependencies.a]
            path = "a"
            [dependencies.b]
            path = "b"
        "#)
        .file("src/lib.rs", "extern crate a; extern crate b;")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            authors = []
            version = "0.0.1"
            [dependencies.b]
            path = "../b"
        "#)
        .file("a/src/lib.rs", "extern crate b;")
        .file("b/Cargo.toml", r#"
            [package]
            name = "b"
            authors = []
            version = "0.0.1"
        "#)
        .file("b/src/lib.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0));

    File::create(&p.root().join("b/src/lib.rs")).unwrap().write_all(br#"
        pub fn b() {}
    "#).unwrap();

    assert_that(p.cargo("build").arg("-pb"),
                execs().with_status(0));

    File::create(&p.root().join("src/lib.rs")).unwrap().write_all(br#"
        extern crate a;
        extern crate b;
        pub fn toplevel() {}
    "#).unwrap();

    assert_that(p.cargo("build"),
                execs().with_status(0));
});

test!(changing_features_is_ok {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            authors = []
            version = "0.0.1"

            [features]
            foo = []
        "#)
        .file("src/lib.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0)
                       .with_stdout("\
[..]Compiling foo v0.0.1 ([..])
"));

    assert_that(p.cargo("build").arg("--features").arg("foo"),
                execs().with_status(0)
                       .with_stdout("\
[..]Compiling foo v0.0.1 ([..])
"));

    assert_that(p.cargo("build"),
                execs().with_status(0)
                       .with_stdout("\
[..]Compiling foo v0.0.1 ([..])
"));

    assert_that(p.cargo("build"),
                execs().with_status(0)
                       .with_stdout(""));
});

test!(rebuild_tests_if_lib_changes {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "pub fn foo() {}")
        .file("tests/foo.rs", r#"
            extern crate foo;
            #[test]
            fn test() { foo::foo(); }
        "#);

    assert_that(p.cargo_process("build"),
                execs().with_status(0));
    assert_that(p.cargo("test"),
                execs().with_status(0));

    File::create(&p.root().join("src/lib.rs")).unwrap();
    p.root().move_into_the_past().unwrap();
    p.root().join("target").move_into_the_past().unwrap();

    assert_that(p.cargo("build"),
                execs().with_status(0));
    assert_that(p.cargo("test").arg("-v"),
                execs().with_status(101));
});

test!(no_rebuild_transitive_target_deps {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a" }
            [dev-dependencies]
            b = { path = "b" }
        "#)
        .file("src/lib.rs", "")
        .file("tests/foo.rs", "")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []

            [target.foo.dependencies]
            c = { path = "../c" }
        "#)
        .file("a/src/lib.rs", "")
        .file("b/Cargo.toml", r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []

            [dependencies]
            c = { path = "../c" }
        "#)
        .file("b/src/lib.rs", "")
        .file("c/Cargo.toml", r#"
            [package]
            name = "c"
            version = "0.0.1"
            authors = []
        "#)
        .file("c/src/lib.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0));
    assert_that(p.cargo("test").arg("--no-run"),
                execs().with_status(0)
                       .with_stdout(&format!("\
{compiling} c v0.0.1 ([..])
{compiling} b v0.0.1 ([..])
{compiling} foo v0.0.1 ([..])
", compiling = COMPILING)));
});

test!(rerun_if_changed_in_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a" }
        "#)
        .file("src/lib.rs", "")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            build = "build.rs"
        "#)
        .file("a/build.rs", r#"
            fn main() {
                println!("cargo:rerun-if-changed=build.rs");
            }
        "#)
        .file("a/src/lib.rs", "");

    assert_that(p.cargo_process("build"),
                execs().with_status(0));
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));
});
