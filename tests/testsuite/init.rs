use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use support;

use support::{paths, Execs};

fn cargo_process(s: &str) -> Execs {
    let mut execs = support::cargo_process(s);
    execs.cwd(&paths::root()).env("HOME", &paths::home());
    execs
}

#[test]
fn simple_lib() {
    cargo_process("init --lib --vcs none --edition 2015")
        .env("USER", "foo")
        .with_stderr("[CREATED] library package")
        .run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(!paths::root().join(".gitignore").is_file());

    cargo_process("build").run();
}

#[test]
fn simple_bin() {
    let path = paths::root().join("foo");
    fs::create_dir(&path).unwrap();
    cargo_process("init --bin --vcs none --edition 2015")
        .env("USER", "foo")
        .cwd(&path)
        .with_stderr("[CREATED] binary (application) package")
        .run();

    assert!(paths::root().join("foo/Cargo.toml").is_file());
    assert!(paths::root().join("foo/src/main.rs").is_file());

    cargo_process("build").cwd(&path).run();
    assert!(
        paths::root()
            .join(&format!("foo/target/debug/foo{}", env::consts::EXE_SUFFIX))
            .is_file()
    );
}

#[test]
fn both_lib_and_bin() {
    cargo_process("init --lib --bin")
        .env("USER", "foo")
        .with_status(101)
        .with_stderr("[ERROR] can't specify both lib and binary outputs")
        .run();
}

fn bin_already_exists(explicit: bool, rellocation: &str) {
    let path = paths::root().join("foo");
    fs::create_dir_all(&path.join("src")).unwrap();

    let sourcefile_path = path.join(rellocation);

    let content = br#"
        fn main() {
            println!("Hello, world 2!");
        }
    "#;

    File::create(&sourcefile_path)
        .unwrap()
        .write_all(content)
        .unwrap();

    if explicit {
        cargo_process("init --bin --vcs none")
            .env("USER", "foo")
            .cwd(&path)
            .run();
    } else {
        cargo_process("init --vcs none")
            .env("USER", "foo")
            .cwd(&path)
            .run();
    }

    assert!(paths::root().join("foo/Cargo.toml").is_file());
    assert!(!paths::root().join("foo/src/lib.rs").is_file());

    // Check that our file is not overwritten
    let mut new_content = Vec::new();
    File::open(&sourcefile_path)
        .unwrap()
        .read_to_end(&mut new_content)
        .unwrap();
    assert_eq!(Vec::from(content as &[u8]), new_content);
}

#[test]
fn bin_already_exists_explicit() {
    bin_already_exists(true, "src/main.rs")
}

#[test]
fn bin_already_exists_implicit() {
    bin_already_exists(false, "src/main.rs")
}

#[test]
fn bin_already_exists_explicit_nosrc() {
    bin_already_exists(true, "main.rs")
}

#[test]
fn bin_already_exists_implicit_nosrc() {
    bin_already_exists(false, "main.rs")
}

#[test]
fn bin_already_exists_implicit_namenosrc() {
    bin_already_exists(false, "foo.rs")
}

#[test]
fn bin_already_exists_implicit_namesrc() {
    bin_already_exists(false, "src/foo.rs")
}

#[test]
fn confused_by_multiple_lib_files() {
    let path = paths::root().join("foo");
    fs::create_dir_all(&path.join("src")).unwrap();

    let sourcefile_path1 = path.join("src/lib.rs");

    File::create(&sourcefile_path1)
        .unwrap()
        .write_all(br#"fn qqq () { println!("Hello, world 2!"); }"#)
        .unwrap();

    let sourcefile_path2 = path.join("lib.rs");

    File::create(&sourcefile_path2)
        .unwrap()
        .write_all(br#" fn qqq () { println!("Hello, world 3!"); }"#)
        .unwrap();

    cargo_process("init --vcs none").env("USER", "foo").cwd(&path).with_status(101).with_stderr(
            "[ERROR] cannot have a package with multiple libraries, found both `src/lib.rs` and `lib.rs`",
        )
        .run();

    assert!(!paths::root().join("foo/Cargo.toml").is_file());
}

#[test]
fn multibin_project_name_clash() {
    let path = paths::root().join("foo");
    fs::create_dir(&path).unwrap();

    let sourcefile_path1 = path.join("foo.rs");

    File::create(&sourcefile_path1)
        .unwrap()
        .write_all(br#"fn main () { println!("Hello, world 2!"); }"#)
        .unwrap();

    let sourcefile_path2 = path.join("main.rs");

    File::create(&sourcefile_path2)
        .unwrap()
        .write_all(br#"fn main () { println!("Hello, world 3!"); }"#)
        .unwrap();

    cargo_process("init --lib --vcs none")
        .env("USER", "foo")
        .cwd(&path)
        .with_status(101)
        .with_stderr(
            "\
[ERROR] multiple possible binary sources found:
  main.rs
  foo.rs
cannot automatically generate Cargo.toml as the main target would be ambiguous
",
        ).run();

    assert!(!paths::root().join("foo/Cargo.toml").is_file());
}

fn lib_already_exists(rellocation: &str) {
    let path = paths::root().join("foo");
    fs::create_dir_all(&path.join("src")).unwrap();

    let sourcefile_path = path.join(rellocation);

    let content = br#"
        pub fn qqq() {}
    "#;

    File::create(&sourcefile_path)
        .unwrap()
        .write_all(content)
        .unwrap();

    cargo_process("init --vcs none")
        .env("USER", "foo")
        .cwd(&path)
        .run();

    assert!(paths::root().join("foo/Cargo.toml").is_file());
    assert!(!paths::root().join("foo/src/main.rs").is_file());

    // Check that our file is not overwritten
    let mut new_content = Vec::new();
    File::open(&sourcefile_path)
        .unwrap()
        .read_to_end(&mut new_content)
        .unwrap();
    assert_eq!(Vec::from(content as &[u8]), new_content);
}

#[test]
fn lib_already_exists_src() {
    lib_already_exists("src/lib.rs");
}

#[test]
fn lib_already_exists_nosrc() {
    lib_already_exists("lib.rs");
}

#[test]
fn simple_git() {
    cargo_process("init --lib --vcs git")
        .env("USER", "foo")
        .run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(paths::root().join(".git").is_dir());
    assert!(paths::root().join(".gitignore").is_file());
}

#[test]
fn auto_git() {
    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(paths::root().join(".git").is_dir());
    assert!(paths::root().join(".gitignore").is_file());
}

#[test]
fn invalid_dir_name() {
    let foo = &paths::root().join("foo.bar");
    fs::create_dir_all(&foo).unwrap();
    cargo_process("init")
        .cwd(foo.clone())
        .env("USER", "foo")
        .with_status(101)
        .with_stderr(
            "\
[ERROR] Invalid character `.` in crate name: `foo.bar`
use --name to override crate name
",
        ).run();

    assert!(!foo.join("Cargo.toml").is_file());
}

#[test]
fn reserved_name() {
    let test = &paths::root().join("test");
    fs::create_dir_all(&test).unwrap();
    cargo_process("init")
        .cwd(test.clone())
        .env("USER", "foo")
        .with_status(101)
        .with_stderr(
            "\
[ERROR] The name `test` cannot be used as a crate name\n\
use --name to override crate name
",
        ).run();

    assert!(!test.join("Cargo.toml").is_file());
}

#[test]
fn git_autodetect() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(paths::root().join(".git").is_dir());
    assert!(paths::root().join(".gitignore").is_file());
}

#[test]
fn mercurial_autodetect() {
    fs::create_dir(&paths::root().join(".hg")).unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(!paths::root().join(".git").is_dir());
    assert!(paths::root().join(".hgignore").is_file());
}

#[test]
fn gitignore_appended_not_replaced() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    File::create(&paths::root().join(".gitignore"))
        .unwrap()
        .write_all(b"qqqqqq\n")
        .unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join("Cargo.toml").is_file());
    assert!(paths::root().join("src/lib.rs").is_file());
    assert!(paths::root().join(".git").is_dir());
    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(contents.contains(r#"qqqqqq"#));
}

#[test]
fn gitignore_added_newline_in_existing() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    File::create(&paths::root().join(".gitignore"))
        .unwrap()
        .write_all(b"first")
        .unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(contents.starts_with("first\n"));
}

#[test]
fn gitignore_no_newline_in_new() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(!contents.starts_with('\n'));
}

#[test]
fn mercurial_added_newline_in_existing() {
    fs::create_dir(&paths::root().join(".hg")).unwrap();

    File::create(&paths::root().join(".hgignore"))
        .unwrap()
        .write_all(b"first")
        .unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join(".hgignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".hgignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(contents.starts_with("first\n"));
}

#[test]
fn mercurial_no_newline_in_new() {
    fs::create_dir(&paths::root().join(".hg")).unwrap();

    cargo_process("init --lib").env("USER", "foo").run();

    assert!(paths::root().join(".hgignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".hgignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(!contents.starts_with('\n'));
}

#[test]
fn cargo_lock_gitignored_if_lib1() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    cargo_process("init --lib --vcs git")
        .env("USER", "foo")
        .run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(contents.contains(r#"Cargo.lock"#));
}

#[test]
fn cargo_lock_gitignored_if_lib2() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    File::create(&paths::root().join("lib.rs"))
        .unwrap()
        .write_all(br#""#)
        .unwrap();

    cargo_process("init --vcs git").env("USER", "foo").run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(contents.contains(r#"Cargo.lock"#));
}

#[test]
fn cargo_lock_not_gitignored_if_bin1() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    cargo_process("init --vcs git --bin")
        .env("USER", "foo")
        .run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(!contents.contains(r#"Cargo.lock"#));
}

#[test]
fn cargo_lock_not_gitignored_if_bin2() {
    fs::create_dir(&paths::root().join(".git")).unwrap();

    File::create(&paths::root().join("main.rs"))
        .unwrap()
        .write_all(br#""#)
        .unwrap();

    cargo_process("init --vcs git").env("USER", "foo").run();

    assert!(paths::root().join(".gitignore").is_file());

    let mut contents = String::new();
    File::open(&paths::root().join(".gitignore"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert!(!contents.contains(r#"Cargo.lock"#));
}

#[test]
fn with_argument() {
    cargo_process("init foo --vcs none")
        .env("USER", "foo")
        .run();
    assert!(paths::root().join("foo/Cargo.toml").is_file());
}

#[test]
fn unknown_flags() {
    cargo_process("init foo --flag")
        .with_status(1)
        .with_stderr_contains(
            "error: Found argument '--flag' which wasn't expected, or isn't valid in this context",
        ).run();
}

#[cfg(not(windows))]
#[test]
fn no_filename() {
    cargo_process("init /")
        .with_status(101)
        .with_stderr(
            "[ERROR] cannot auto-detect package name from path \"/\" ; use --name to override"
                .to_string(),
        ).run();
}
