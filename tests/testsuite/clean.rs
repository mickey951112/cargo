use std::env;

use support::{basic_manifest, basic_bin_manifest, execs, git, main_file, project};
use support::registry::Package;
use support::hamcrest::{assert_that, existing_dir, existing_file, is_not};

#[test]
fn cargo_clean_simple() {
    let p = project()
        .file("Cargo.toml", &basic_bin_manifest("foo"))
        .file("src/foo.rs", &main_file(r#""i am foo""#, &[]))
        .build();

    assert_that(p.cargo("build"), execs());
    assert_that(&p.build_dir(), existing_dir());

    assert_that(p.cargo("clean"), execs());
    assert_that(&p.build_dir(), is_not(existing_dir()));
}

#[test]
fn different_dir() {
    let p = project()
        .file("Cargo.toml", &basic_bin_manifest("foo"))
        .file("src/foo.rs", &main_file(r#""i am foo""#, &[]))
        .file("src/bar/a.rs", "")
        .build();

    assert_that(p.cargo("build"), execs());
    assert_that(&p.build_dir(), existing_dir());

    assert_that(
        p.cargo("clean").cwd(&p.root().join("src")),
        execs().with_stdout(""),
    );
    assert_that(&p.build_dir(), is_not(existing_dir()));
}

#[test]
fn clean_multiple_packages() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.d1]
                path = "d1"
            [dependencies.d2]
                path = "d2"

            [[bin]]
                name = "foo"
        "#,
        )
        .file("src/foo.rs", &main_file(r#""i am foo""#, &[]))
        .file("d1/Cargo.toml", &basic_bin_manifest("d1"))
        .file("d1/src/main.rs", "fn main() { println!(\"d1\"); }")
        .file("d2/Cargo.toml", &basic_bin_manifest("d2"))
        .file("d2/src/main.rs", "fn main() { println!(\"d2\"); }")
        .build();

    assert_that(p.cargo("build -p d1 -p d2 -p foo"), execs());

    let d1_path = &p.build_dir()
        .join("debug")
        .join(format!("d1{}", env::consts::EXE_SUFFIX));
    let d2_path = &p.build_dir()
        .join("debug")
        .join(format!("d2{}", env::consts::EXE_SUFFIX));

    assert_that(&p.bin("foo"), existing_file());
    assert_that(d1_path, existing_file());
    assert_that(d2_path, existing_file());

    assert_that(
        p.cargo("clean -p d1 -p d2").cwd(&p.root().join("src")),
        execs().with_stdout(""),
    );
    assert_that(&p.bin("foo"), existing_file());
    assert_that(d1_path, is_not(existing_file()));
    assert_that(d2_path, is_not(existing_file()));
}

#[test]
fn clean_release() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a" }
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("a", "0.0.1"))
        .file("a/src/lib.rs", "")
        .build();

    assert_that(p.cargo("build --release"), execs());

    assert_that(
        p.cargo("clean -p foo"),
        execs(),
    );
    assert_that(
        p.cargo("build --release"),
        execs().with_stdout(""),
    );

    assert_that(
        p.cargo("clean -p foo --release"),
        execs(),
    );
    assert_that(
        p.cargo("build --release"),
        execs().with_stderr(
            "\
[COMPILING] foo v0.0.1 ([..])
[FINISHED] release [optimized] target(s) in [..]
",
        ),
    );
}

#[test]
fn clean_doc() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a" }
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("a", "0.0.1"))
        .file("a/src/lib.rs", "")
        .build();

    assert_that(p.cargo("doc"), execs());

    let doc_path = &p.build_dir().join("doc");

    assert_that(doc_path, existing_dir());

    assert_that(p.cargo("clean --doc"), execs());

    assert_that(doc_path, is_not(existing_dir()));
    assert_that(p.build_dir(), existing_dir());
}

#[test]
fn build_script() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
            build = "build.rs"
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file(
            "build.rs",
            r#"
            use std::path::PathBuf;
            use std::env;

            fn main() {
                let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
                if env::var("FIRST").is_ok() {
                    std::fs::File::create(out.join("out")).unwrap();
                } else {
                    assert!(!std::fs::metadata(out.join("out")).is_ok());
                }
            }
        "#,
        )
        .file("a/src/lib.rs", "")
        .build();

    assert_that(p.cargo("build").env("FIRST", "1"), execs());
    assert_that(
        p.cargo("clean -p foo"),
        execs(),
    );
    assert_that(
        p.cargo("build -v"),
        execs().with_stderr(
            "\
[COMPILING] foo v0.0.1 ([..])
[RUNNING] `rustc [..] build.rs [..]`
[RUNNING] `[..]build-script-build`
[RUNNING] `rustc [..] src/main.rs [..]`
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        ),
    );
}

#[test]
fn clean_git() {
    let git = git::new("dep", |project| {
        project
            .file("Cargo.toml", &basic_manifest("dep", "0.5.0"))
            .file("src/lib.rs", "")
    }).unwrap();

    let p = project()
        .file(
            "Cargo.toml",
            &format!(
                r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            dep = {{ git = '{}' }}
        "#,
                git.url()
            ),
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    assert_that(p.cargo("build"), execs());
    assert_that(
        p.cargo("clean -p dep"),
        execs().with_stdout(""),
    );
    assert_that(p.cargo("build"), execs());
}

#[test]
fn registry() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "0.1"
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    Package::new("bar", "0.1.0").publish();

    assert_that(p.cargo("build"), execs());
    assert_that(
        p.cargo("clean -p bar"),
        execs().with_stdout(""),
    );
    assert_that(p.cargo("build"), execs());
}

#[test]
fn clean_verbose() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
        [package]
        name = "foo"
        version = "0.0.1"

        [dependencies]
        bar = "0.1"
    "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    Package::new("bar", "0.1.0").publish();

    assert_that(p.cargo("build"), execs());
    assert_that(
        p.cargo("clean -p bar --verbose"),
        execs().with_stderr(
            "\
[REMOVING] [..]
[REMOVING] [..]
",
        ),
    );
    assert_that(p.cargo("build"), execs());
}
