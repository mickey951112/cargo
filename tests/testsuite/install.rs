use std::fs::{self, File, OpenOptions};
use std::io::prelude::*;
use support;

use git2;
use support::cross_compile;
use support::git;
use support::install::{assert_has_installed_exe, assert_has_not_installed_exe, cargo_home};
use support::paths;
use support::registry::Package;
use support::{basic_manifest, cargo_process, project};

fn pkg(name: &str, vers: &str) {
    Package::new(name, vers)
        .file("src/lib.rs", "")
        .file(
            "src/main.rs",
            &format!("extern crate {}; fn main() {{}}", name),
        ).publish();
}

#[test]
fn simple() {
    pkg("foo", "0.0.1");

    cargo_process("install foo")
        .with_stderr(
            "\
[UPDATING] registry `[..]`
[DOWNLOADING] foo v0.0.1 (registry [..])
[INSTALLING] foo v0.0.1
[COMPILING] foo v0.0.1
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/foo[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();
    assert_has_installed_exe(cargo_home(), "foo");

    cargo_process("uninstall foo")
        .with_stderr("[REMOVING] CWD/home/.cargo/bin/foo[EXE]")
        .run();
    assert_has_not_installed_exe(cargo_home(), "foo");
}

#[test]
fn multiple_pkgs() {
    pkg("foo", "0.0.1");
    pkg("bar", "0.0.2");

    cargo_process("install foo bar baz")
        .with_status(101)
        .with_stderr(
            "\
[UPDATING] registry `[..]`
[DOWNLOADING] foo v0.0.1 (registry `CWD/registry`)
[INSTALLING] foo v0.0.1
[COMPILING] foo v0.0.1
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/foo[EXE]
[DOWNLOADING] bar v0.0.2 (registry `CWD/registry`)
[INSTALLING] bar v0.0.2
[COMPILING] bar v0.0.2
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/bar[EXE]
error: could not find `baz` in registry `[..]`
[SUMMARY] Successfully installed foo, bar! Failed to install baz (see error(s) above).
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
error: some crates failed to install
",
        ).run();
    assert_has_installed_exe(cargo_home(), "foo");
    assert_has_installed_exe(cargo_home(), "bar");

    cargo_process("uninstall foo bar")
        .with_stderr(
            "\
[REMOVING] CWD/home/.cargo/bin/foo[EXE]
[REMOVING] CWD/home/.cargo/bin/bar[EXE]
[SUMMARY] Successfully uninstalled foo, bar!
",
        ).run();

    assert_has_not_installed_exe(cargo_home(), "foo");
    assert_has_not_installed_exe(cargo_home(), "bar");
}

#[test]
fn pick_max_version() {
    pkg("foo", "0.1.0");
    pkg("foo", "0.2.0");
    pkg("foo", "0.2.1");
    pkg("foo", "0.2.1-pre.1");
    pkg("foo", "0.3.0-pre.2");

    cargo_process("install foo")
        .with_stderr(
            "\
[UPDATING] registry `[..]`
[DOWNLOADING] foo v0.2.1 (registry [..])
[INSTALLING] foo v0.2.1
[COMPILING] foo v0.2.1
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/foo[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn installs_beta_version_by_explicit_name_from_git() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.3.0-beta.1"))
        .file("src/main.rs", "fn main() {}")
        .build();

    cargo_process("install --git")
        .arg(p.url().to_string())
        .arg("foo")
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn missing() {
    pkg("foo", "0.0.1");
    cargo_process("install bar")
        .with_status(101)
        .with_stderr(
            "\
[UPDATING] registry [..]
[ERROR] could not find `bar` in registry `[..]`
",
        ).run();
}

#[test]
fn bad_version() {
    pkg("foo", "0.0.1");
    cargo_process("install foo --vers=0.2.0")
        .with_status(101)
        .with_stderr(
            "\
[UPDATING] registry [..]
[ERROR] could not find `foo` in registry `[..]` with version `=0.2.0`
",
        ).run();
}

#[test]
fn no_crate() {
    cargo_process("install")
        .with_status(101)
        .with_stderr(
            "\
[ERROR] `[..]` is not a crate root; specify a crate to install [..]

Caused by:
  failed to read `[..]Cargo.toml`

Caused by:
  [..] (os error [..])
",
        ).run();
}

#[test]
fn install_location_precedence() {
    pkg("foo", "0.0.1");

    let root = paths::root();
    let t1 = root.join("t1");
    let t2 = root.join("t2");
    let t3 = root.join("t3");
    let t4 = cargo_home();

    fs::create_dir(root.join(".cargo")).unwrap();
    File::create(root.join(".cargo/config"))
        .unwrap()
        .write_all(
            format!(
                "\
        [install]
        root = '{}'
    ",
                t3.display()
            ).as_bytes(),
        ).unwrap();

    println!("install --root");

    cargo_process("install foo --root")
        .arg(&t1)
        .env("CARGO_INSTALL_ROOT", &t2)
        .run();
    assert_has_installed_exe(&t1, "foo");
    assert_has_not_installed_exe(&t2, "foo");

    println!("install CARGO_INSTALL_ROOT");

    cargo_process("install foo")
        .env("CARGO_INSTALL_ROOT", &t2)
        .run();
    assert_has_installed_exe(&t2, "foo");
    assert_has_not_installed_exe(&t3, "foo");

    println!("install install.root");

    cargo_process("install foo").run();
    assert_has_installed_exe(&t3, "foo");
    assert_has_not_installed_exe(&t4, "foo");

    fs::remove_file(root.join(".cargo/config")).unwrap();

    println!("install cargo home");

    cargo_process("install foo").run();
    assert_has_installed_exe(&t4, "foo");
}

#[test]
fn install_path() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    cargo_process("install --path").arg(p.root()).run();
    assert_has_installed_exe(cargo_home(), "foo");
    p.cargo("install --path .")
        .with_status(101)
        .with_stderr(
            "\
[INSTALLING] foo v0.0.1 [..]
[ERROR] binary `foo[..]` already exists in destination as part of `foo v0.0.1 [..]`
Add --force to overwrite
",
        ).run();
}

#[test]
fn multiple_crates_error() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.1.0"))
        .file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/main.rs", "fn main() {}")
        .build();

    cargo_process("install --git")
        .arg(p.url().to_string())
        .with_status(101)
        .with_stderr(
            "\
[UPDATING] git repository [..]
[ERROR] multiple packages with binaries found: bar, foo
",
        ).run();
}

#[test]
fn multiple_crates_select() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.1.0"))
        .file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/main.rs", "fn main() {}")
        .build();

    cargo_process("install --git")
        .arg(p.url().to_string())
        .arg("foo")
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
    assert_has_not_installed_exe(cargo_home(), "bar");

    cargo_process("install --git")
        .arg(p.url().to_string())
        .arg("bar")
        .run();
    assert_has_installed_exe(cargo_home(), "bar");
}

#[test]
fn multiple_crates_auto_binaries() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [dependencies]
            bar = { path = "a" }
        "#,
        ).file("src/main.rs", "extern crate bar; fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/lib.rs", "")
        .build();

    cargo_process("install --path").arg(p.root()).run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn multiple_crates_auto_examples() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [dependencies]
            bar = { path = "a" }
        "#,
        ).file("src/lib.rs", "extern crate bar;")
        .file(
            "examples/foo.rs",
            "
            extern crate bar;
            extern crate foo;
            fn main() {}
        ",
        ).file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/lib.rs", "")
        .build();

    cargo_process("install --path")
        .arg(p.root())
        .arg("--example=foo")
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn no_binaries_or_examples() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [dependencies]
            bar = { path = "a" }
        "#,
        ).file("src/lib.rs", "")
        .file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/lib.rs", "")
        .build();

    cargo_process("install --path")
        .arg(p.root())
        .with_status(101)
        .with_stderr("[ERROR] no packages found with binaries or examples")
        .run();
}

#[test]
fn no_binaries() {
    let p = project()
        .file("src/lib.rs", "")
        .file("examples/foo.rs", "fn main() {}")
        .build();

    cargo_process("install --path")
        .arg(p.root())
        .arg("foo")
        .with_status(101)
        .with_stderr(
            "\
[INSTALLING] foo [..]
[ERROR] specified package has no binaries
",
        ).run();
}

#[test]
fn examples() {
    let p = project()
        .file("src/lib.rs", "")
        .file("examples/foo.rs", "extern crate foo; fn main() {}")
        .build();

    cargo_process("install --path")
        .arg(p.root())
        .arg("--example=foo")
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn install_twice() {
    let p = project()
        .file("src/bin/foo-bin1.rs", "fn main() {}")
        .file("src/bin/foo-bin2.rs", "fn main() {}")
        .build();

    cargo_process("install --path").arg(p.root()).run();
    cargo_process("install --path")
        .arg(p.root())
        .with_status(101)
        .with_stderr(
            "\
[INSTALLING] foo v0.0.1 [..]
[ERROR] binary `foo-bin1[..]` already exists in destination as part of `foo v0.0.1 ([..])`
binary `foo-bin2[..]` already exists in destination as part of `foo v0.0.1 ([..])`
Add --force to overwrite
",
        ).run();
}

#[test]
fn install_force() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    cargo_process("install --path").arg(p.root()).run();

    let p = project()
        .at("foo2")
        .file("Cargo.toml", &basic_manifest("foo", "0.2.0"))
        .file("src/main.rs", "fn main() {}")
        .build();

    cargo_process("install --force --path")
        .arg(p.root())
        .with_stderr(
            "\
[INSTALLING] foo v0.2.0 ([..])
[COMPILING] foo v0.2.0 ([..])
[FINISHED] release [optimized] target(s) in [..]
[REPLACING] CWD/home/.cargo/bin/foo[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();

    cargo_process("install --list")
        .with_stdout(
            "\
foo v0.2.0 ([..]):
    foo[..]
",
        ).run();
}

#[test]
fn install_force_partial_overlap() {
    let p = project()
        .file("src/bin/foo-bin1.rs", "fn main() {}")
        .file("src/bin/foo-bin2.rs", "fn main() {}")
        .build();

    cargo_process("install --path").arg(p.root()).run();

    let p = project()
        .at("foo2")
        .file("Cargo.toml", &basic_manifest("foo", "0.2.0"))
        .file("src/bin/foo-bin2.rs", "fn main() {}")
        .file("src/bin/foo-bin3.rs", "fn main() {}")
        .build();

    cargo_process("install --force --path")
        .arg(p.root())
        .with_stderr(
            "\
[INSTALLING] foo v0.2.0 ([..])
[COMPILING] foo v0.2.0 ([..])
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/foo-bin3[EXE]
[REPLACING] CWD/home/.cargo/bin/foo-bin2[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();

    cargo_process("install --list")
        .with_stdout(
            "\
foo v0.0.1 ([..]):
    foo-bin1[..]
foo v0.2.0 ([..]):
    foo-bin2[..]
    foo-bin3[..]
",
        ).run();
}

#[test]
fn install_force_bin() {
    let p = project()
        .file("src/bin/foo-bin1.rs", "fn main() {}")
        .file("src/bin/foo-bin2.rs", "fn main() {}")
        .build();

    cargo_process("install --path").arg(p.root()).run();

    let p = project()
        .at("foo2")
        .file("Cargo.toml", &basic_manifest("foo", "0.2.0"))
        .file("src/bin/foo-bin1.rs", "fn main() {}")
        .file("src/bin/foo-bin2.rs", "fn main() {}")
        .build();

    cargo_process("install --force --bin foo-bin2 --path")
        .arg(p.root())
        .with_stderr(
            "\
[INSTALLING] foo v0.2.0 ([..])
[COMPILING] foo v0.2.0 ([..])
[FINISHED] release [optimized] target(s) in [..]
[REPLACING] CWD/home/.cargo/bin/foo-bin2[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();

    cargo_process("install --list")
        .with_stdout(
            "\
foo v0.0.1 ([..]):
    foo-bin1[..]
foo v0.2.0 ([..]):
    foo-bin2[..]
",
        ).run();
}

#[test]
fn compile_failure() {
    let p = project().file("src/main.rs", "").build();

    cargo_process("install --path")
        .arg(p.root())
        .with_status(101)
        .with_stderr_contains(
            "\
[ERROR] failed to compile `foo v0.0.1 ([..])`, intermediate artifacts can be \
    found at `[..]target`

Caused by:
  Could not compile `foo`.

To learn more, run the command again with --verbose.
",
        ).run();
}

#[test]
fn git_repo() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.1.0"))
        .file("src/main.rs", "fn main() {}")
        .build();

    // use `--locked` to test that we don't even try to write a lockfile
    cargo_process("install --locked --git")
        .arg(p.url().to_string())
        .with_stderr(
            "\
[UPDATING] git repository `[..]`
[INSTALLING] foo v0.1.0 ([..])
[COMPILING] foo v0.1.0 ([..])
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] CWD/home/.cargo/bin/foo[EXE]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();
    assert_has_installed_exe(cargo_home(), "foo");
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn list() {
    pkg("foo", "0.0.1");
    pkg("bar", "0.2.1");
    pkg("bar", "0.2.2");

    cargo_process("install --list").with_stdout("").run();

    cargo_process("install bar --vers =0.2.1").run();
    cargo_process("install foo").run();
    cargo_process("install --list")
        .with_stdout(
            "\
bar v0.2.1:
    bar[..]
foo v0.0.1:
    foo[..]
",
        ).run();
}

#[test]
fn list_error() {
    pkg("foo", "0.0.1");
    cargo_process("install foo").run();
    cargo_process("install --list")
        .with_stdout(
            "\
foo v0.0.1:
    foo[..]
",
        ).run();
    let mut worldfile_path = cargo_home();
    worldfile_path.push(".crates.toml");
    let mut worldfile = OpenOptions::new()
        .write(true)
        .open(worldfile_path)
        .expect(".crates.toml should be there");
    worldfile.write_all(b"\x00").unwrap();
    drop(worldfile);
    cargo_process("install --list --verbose")
        .with_status(101)
        .with_stderr(
            "\
[ERROR] failed to parse crate metadata at `[..]`

Caused by:
  invalid TOML found for metadata

Caused by:
  unexpected character[..]
",
        ).run();
}

#[test]
fn uninstall_pkg_does_not_exist() {
    cargo_process("uninstall foo")
        .with_status(101)
        .with_stderr("[ERROR] package id specification `foo` matched no packages")
        .run();
}

#[test]
fn uninstall_bin_does_not_exist() {
    pkg("foo", "0.0.1");

    cargo_process("install foo").run();
    cargo_process("uninstall foo --bin=bar")
        .with_status(101)
        .with_stderr("[ERROR] binary `bar[..]` not installed as part of `foo v0.0.1`")
        .run();
}

#[test]
fn uninstall_piecemeal() {
    let p = project()
        .file("src/bin/foo.rs", "fn main() {}")
        .file("src/bin/bar.rs", "fn main() {}")
        .build();

    cargo_process("install --path").arg(p.root()).run();
    assert_has_installed_exe(cargo_home(), "foo");
    assert_has_installed_exe(cargo_home(), "bar");

    cargo_process("uninstall foo --bin=bar")
        .with_stderr("[REMOVING] [..]bar[..]")
        .run();

    assert_has_installed_exe(cargo_home(), "foo");
    assert_has_not_installed_exe(cargo_home(), "bar");

    cargo_process("uninstall foo --bin=foo")
        .with_stderr("[REMOVING] [..]foo[..]")
        .run();
    assert_has_not_installed_exe(cargo_home(), "foo");

    cargo_process("uninstall foo")
        .with_status(101)
        .with_stderr("[ERROR] package id specification `foo` matched no packages")
        .run();
}

#[test]
fn subcommand_works_out_of_the_box() {
    Package::new("cargo-foo", "1.0.0")
        .file("src/main.rs", r#"fn main() { println!("bar"); }"#)
        .publish();
    cargo_process("install cargo-foo").run();
    cargo_process("foo").with_stdout("bar\n").run();
    cargo_process("--list")
        .with_stdout_contains("    foo\n")
        .run();
}

#[test]
fn installs_from_cwd_by_default() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    p.cargo("install")
        .with_stderr_contains(
            "warning: Using `cargo install` to install the binaries for the \
             project in current working directory is deprecated, \
             use `cargo install --path .` instead. \
             Use `cargo build` if you want to simply build the package.",
        ).run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn installs_from_cwd_with_2018_warnings() {
    if !support::is_nightly() {
        // Stable rust won't have the edition option.  Remove this once it
        // is stabilized.
        return;
    }
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            cargo-features = ["edition"]

            [package]
            name = "foo"
            version = "0.1.0"
            authors = []
            edition = "2018"
        "#,
        ).file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("install")
        .masquerade_as_nightly_cargo()
        .with_status(101)
        .with_stderr_contains(
            "error: Using `cargo install` to install the binaries for the \
             project in current working directory is no longer supported, \
             use `cargo install --path .` instead. \
             Use `cargo build` if you want to simply build the package.",
        ).run();
    assert_has_not_installed_exe(cargo_home(), "foo");
}

#[test]
fn uninstall_cwd() {
    let p = project().file("src/main.rs", "fn main() {}").build();
    p.cargo("install --path .")
        .with_stderr(&format!(
            "\
[INSTALLING] foo v0.0.1 (CWD)
[COMPILING] foo v0.0.1 (CWD)
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] {home}/bin/foo[EXE]
warning: be sure to add `{home}/bin` to your PATH to be able to run the installed binaries",
            home = cargo_home().display(),
        )).run();
    assert_has_installed_exe(cargo_home(), "foo");

    p.cargo("uninstall")
        .with_stdout("")
        .with_stderr(&format!(
            "\
             [REMOVING] {home}/bin/foo[EXE]",
            home = cargo_home().display()
        )).run();
    assert_has_not_installed_exe(cargo_home(), "foo");
}

#[test]
fn uninstall_cwd_not_installed() {
    let p = project().file("src/main.rs", "fn main() {}").build();
    p.cargo("uninstall")
        .with_status(101)
        .with_stdout("")
        .with_stderr(
            "\
             error: package `foo v0.0.1 (CWD)` is not installed",
        ).run();
}

#[test]
fn uninstall_cwd_no_project() {
    let err_msg = if cfg!(windows) {
        "The system cannot find the file specified."
    } else {
        "No such file or directory"
    };
    cargo_process("uninstall")
        .with_status(101)
        .with_stdout("")
        .with_stderr(format!(
            "\
[ERROR] failed to read `CWD/Cargo.toml`

Caused by:
  {err_msg} (os error 2)",
            err_msg = err_msg,
        )).run();
}

#[test]
fn do_not_rebuilds_on_local_install() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    p.cargo("build --release").run();
    cargo_process("install --path")
        .arg(p.root())
        .with_stderr(
            "[INSTALLING] [..]
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] [..]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();

    assert!(p.build_dir().exists());
    assert!(p.release_bin("foo").exists());
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn reports_unsuccessful_subcommand_result() {
    Package::new("cargo-fail", "1.0.0")
        .file("src/main.rs", "fn main() { panic!(); }")
        .publish();
    cargo_process("install cargo-fail").run();
    cargo_process("--list")
        .with_stdout_contains("    fail\n")
        .run();
    cargo_process("fail")
        .with_status(101)
        .with_stderr_contains("thread '[..]' panicked at 'explicit panic', [..]")
        .run();
}

#[test]
fn git_with_lockfile() {
    let p = git::repo(&paths::root().join("foo"))
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [dependencies]
            bar = { path = "bar" }
        "#,
        ).file("src/main.rs", "fn main() {}")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("bar/src/lib.rs", "fn main() {}")
        .file(
            "Cargo.lock",
            r#"
            [[package]]
            name = "foo"
            version = "0.1.0"
            dependencies = [ "bar 0.1.0" ]

            [[package]]
            name = "bar"
            version = "0.1.0"
        "#,
        ).build();

    cargo_process("install --git")
        .arg(p.url().to_string())
        .run();
}

#[test]
fn q_silences_warnings() {
    let p = project().file("src/main.rs", "fn main() {}").build();

    cargo_process("install -q --path")
        .arg(p.root())
        .with_stderr("")
        .run();
}

#[test]
fn readonly_dir() {
    pkg("foo", "0.0.1");

    let root = paths::root();
    let dir = &root.join("readonly");
    fs::create_dir(root.join("readonly")).unwrap();
    let mut perms = fs::metadata(dir).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(dir, perms).unwrap();

    cargo_process("install foo").cwd(dir).run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn use_path_workspace() {
    Package::new("foo", "1.0.0").publish();
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []

            [workspace]
            members = ["baz"]
        "#,
        ).file("src/main.rs", "fn main() {}")
        .file(
            "baz/Cargo.toml",
            r#"
            [package]
            name = "baz"
            version = "0.1.0"
            authors = []

            [dependencies]
            foo = "1"
        "#,
        ).file("baz/src/lib.rs", "")
        .build();

    p.cargo("build").run();
    let lock = p.read_lockfile();
    p.cargo("install").run();
    let lock2 = p.read_lockfile();
    assert_eq!(lock, lock2, "different lockfiles");
}

#[test]
fn dev_dependencies_no_check() {
    Package::new("foo", "1.0.0").publish();
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []

            [dev-dependencies]
            baz = "1.0.0"
        "#,
        ).file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("build").with_status(101).run();
    p.cargo("install").run();
}

#[test]
fn dev_dependencies_lock_file_untouched() {
    Package::new("foo", "1.0.0").publish();
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [dev-dependencies]
            bar = { path = "a" }
        "#,
        ).file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("a/src/lib.rs", "")
        .build();

    p.cargo("build").run();
    let lock = p.read_lockfile();
    p.cargo("install").run();
    let lock2 = p.read_lockfile();
    assert!(lock == lock2, "different lockfiles");
}

#[test]
fn install_target_native() {
    pkg("foo", "0.1.0");

    cargo_process("install foo --target")
        .arg(support::rustc_host())
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn install_target_foreign() {
    if cross_compile::disabled() {
        return;
    }

    pkg("foo", "0.1.0");

    cargo_process("install foo --target")
        .arg(cross_compile::alternate())
        .run();
    assert_has_installed_exe(cargo_home(), "foo");
}

#[test]
fn vers_precise() {
    pkg("foo", "0.1.1");
    pkg("foo", "0.1.2");

    cargo_process("install foo --vers 0.1.1")
        .with_stderr_contains("[DOWNLOADING] foo v0.1.1 (registry [..])")
        .run();
}

#[test]
fn version_too() {
    pkg("foo", "0.1.1");
    pkg("foo", "0.1.2");

    cargo_process("install foo --version 0.1.1")
        .with_stderr_contains("[DOWNLOADING] foo v0.1.1 (registry [..])")
        .run();
}

#[test]
fn not_both_vers_and_version() {
    pkg("foo", "0.1.1");
    pkg("foo", "0.1.2");

    cargo_process("install foo --version 0.1.1 --vers 0.1.2")
        .with_status(1)
        .with_stderr_contains(
            "\
error: The argument '--version <VERSION>' was provided more than once, \
but cannot be used multiple times
",
        ).run();
}

#[test]
fn legacy_version_requirement() {
    pkg("foo", "0.1.1");

    cargo_process("install foo --vers 0.1")
        .with_stderr_contains(
            "\
warning: the `--vers` provided, `0.1`, is not a valid semver version

historically Cargo treated this as a semver version requirement accidentally
and will continue to do so, but this behavior will be removed eventually
",
        ).run();
}

#[test]
fn test_install_git_cannot_be_a_base_url() {
    cargo_process("install --git github.com:rust-lang-nursery/rustfmt.git").with_status(101).with_stderr("error: invalid url `github.com:rust-lang-nursery/rustfmt.git`: cannot-be-a-base-URLs are not supported").run();
}

#[test]
fn uninstall_multiple_and_specifying_bin() {
    cargo_process("uninstall foo bar --bin baz").with_status(101).with_stderr("error: A binary can only be associated with a single installed package, specifying multiple specs with --bin is redundant.").run();
}

#[test]
fn uninstall_multiple_and_some_pkg_does_not_exist() {
    pkg("foo", "0.0.1");

    cargo_process("install foo").run();

    cargo_process("uninstall foo bar")
        .with_status(101)
        .with_stderr(
            "\
[REMOVING] CWD/home/.cargo/bin/foo[EXE]
error: package id specification `bar` matched no packages
[SUMMARY] Successfully uninstalled foo! Failed to uninstall bar (see error(s) above).
error: some packages failed to uninstall
",
        ).run();

    assert_has_not_installed_exe(cargo_home(), "foo");
    assert_has_not_installed_exe(cargo_home(), "bar");
}

#[test]
fn custom_target_dir_for_git_source() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.1.0"))
        .file("src/main.rs", "fn main() {}")
        .build();

    cargo_process("install --git")
        .arg(p.url().to_string())
        .run();
    assert!(!paths::root().join("target/release").is_dir());

    cargo_process("install --force --git")
        .arg(p.url().to_string())
        .env("CARGO_TARGET_DIR", "target")
        .run();
    assert!(paths::root().join("target/release").is_dir());
}

#[test]
fn install_respects_lock_file() {
    Package::new("bar", "0.1.0").publish();
    Package::new("bar", "0.1.1")
        .file("src/lib.rs", "not rust")
        .publish();
    Package::new("foo", "0.1.0")
        .dep("bar", "0.1")
        .file("src/lib.rs", "")
        .file(
            "src/main.rs",
            "extern crate foo; extern crate bar; fn main() {}",
        ).file(
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
        ).publish();

    cargo_process("install foo").run();
}

#[test]
fn lock_file_path_deps_ok() {
    Package::new("bar", "0.1.0").publish();

    Package::new("foo", "0.1.0")
        .dep("bar", "0.1")
        .file("src/lib.rs", "")
        .file(
            "src/main.rs",
            "extern crate foo; extern crate bar; fn main() {}",
        ).file(
            "Cargo.lock",
            r#"
[[package]]
name = "bar"
version = "0.1.0"

[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "bar 0.1.0",
]
"#,
        ).publish();

    cargo_process("install foo").run();
}

#[test]
fn install_empty_argument() {
    // Bug 5229
    cargo_process("install")
        .arg("")
        .with_status(1)
        .with_stderr_contains(
            "[ERROR] The argument '<crate>...' requires a value but none was supplied",
        ).run();
}

#[test]
fn git_repo_replace() {
    let p = git::repo(&paths::root().join("foo"))
        .file("Cargo.toml", &basic_manifest("foo", "0.1.0"))
        .file("src/main.rs", "fn main() {}")
        .build();
    let repo = git2::Repository::open(&p.root()).unwrap();
    let old_rev = repo.revparse_single("HEAD").unwrap().id();
    cargo_process("install --git")
        .arg(p.url().to_string())
        .run();
    git::commit(&repo);
    let new_rev = repo.revparse_single("HEAD").unwrap().id();
    let mut path = paths::home();
    path.push(".cargo/.crates.toml");

    assert_ne!(old_rev, new_rev);
    assert!(
        fs::read_to_string(path.clone())
            .unwrap()
            .contains(&format!("{}", old_rev))
    );
    cargo_process("install --force --git")
        .arg(p.url().to_string())
        .run();
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains(&format!("{}", new_rev))
    );
}

#[test]
fn workspace_uses_workspace_target_dir() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.1.0"
                authors = []

                [workspace]

                [dependencies]
                bar = { path = 'bar' }
            "#,
        ).file("src/main.rs", "fn main() {}")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("bar/src/main.rs", "fn main() {}")
        .build();

    p.cargo("build --release").cwd(p.root().join("bar")).run();
    cargo_process("install --path")
        .arg(p.root().join("bar"))
        .with_stderr(
            "[INSTALLING] [..]
[FINISHED] release [optimized] target(s) in [..]
[INSTALLING] [..]
warning: be sure to add `[..]` to your PATH to be able to run the installed binaries
",
        ).run();
}

#[test]
fn install_ignores_cargo_config() {
    pkg("bar", "0.0.1");

    let p = project()
        .file(
            ".cargo/config",
            r#"
            [build]
            target = "non-existing-target"
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo("install bar").run();
    assert_has_installed_exe(cargo_home(), "bar");
}
