use std::old_io::{fs, USER_RWX, File, TempDir};
use std::old_io::fs::PathExtensions;
use std::env;

use support::{execs, paths, cargo_dir};
use hamcrest::{assert_that, existing_file, existing_dir, is_not};

use cargo::util::{process, ProcessBuilder};

fn setup() {
}

fn my_process(s: &str) -> ProcessBuilder {
    process(s).unwrap()
        .cwd(paths::root())
        .env("HOME", Some(paths::home()))
}

fn cargo_process(s: &str) -> ProcessBuilder {
    process(cargo_dir().join("cargo")).unwrap().arg(s)
        .cwd(paths::root())
        .env("HOME", Some(paths::home()))
}

test!(simple_lib {
    assert_that(cargo_process("new").arg("foo").arg("--vcs").arg("none")
                                    .env("USER", Some("foo")),
                execs().with_status(0));

    assert_that(&paths::root().join("foo"), existing_dir());
    assert_that(&paths::root().join("foo/Cargo.toml"), existing_file());
    assert_that(&paths::root().join("foo/src/lib.rs"), existing_file());
    assert_that(&paths::root().join("foo/.gitignore"), is_not(existing_file()));

    assert_that(cargo_process("build").cwd(paths::root().join("foo")),
                execs().with_status(0));
});

test!(simple_bin {
    assert_that(cargo_process("new").arg("foo").arg("--bin")
                                    .env("USER", Some("foo")),
                execs().with_status(0));

    assert_that(&paths::root().join("foo"), existing_dir());
    assert_that(&paths::root().join("foo/Cargo.toml"), existing_file());
    assert_that(&paths::root().join("foo/src/main.rs"), existing_file());

    assert_that(cargo_process("build").cwd(paths::root().join("foo")),
                execs().with_status(0));
    assert_that(&paths::root().join(format!("foo/target/foo{}",
                                            env::consts::EXE_SUFFIX)),
                existing_file());
});

test!(simple_git {
    let td = TempDir::new("cargo").unwrap();
    assert_that(cargo_process("new").arg("foo").cwd(td.path().clone())
                                    .env("USER", Some("foo")),
                execs().with_status(0));

    assert_that(td.path(), existing_dir());
    assert_that(&td.path().join("foo/Cargo.toml"), existing_file());
    assert_that(&td.path().join("foo/src/lib.rs"), existing_file());
    assert_that(&td.path().join("foo/.git"), existing_dir());
    assert_that(&td.path().join("foo/.gitignore"), existing_file());

    assert_that(cargo_process("build").cwd(td.path().clone().join("foo")),
                execs().with_status(0));
});

test!(no_argument {
    assert_that(cargo_process("new"),
                execs().with_status(1)
                       .with_stderr("\
Invalid arguments.

Usage:
    cargo new [options] <path>
    cargo new -h | --help
"));
});

test!(existing {
    let dst = paths::root().join("foo");
    fs::mkdir(&dst, USER_RWX).unwrap();
    assert_that(cargo_process("new").arg("foo"),
                execs().with_status(101)
                       .with_stderr(format!("Destination `{}` already exists\n",
                                            dst.display())));
});

test!(invalid_characters {
    assert_that(cargo_process("new").arg("foo.rs"),
                execs().with_status(101)
                       .with_stderr("Invalid character `.` in crate name: `foo.rs`"));
});

test!(finds_author_user {
    // Use a temp dir to make sure we don't pick up .cargo/config somewhere in
    // the hierarchy
    let td = TempDir::new("cargo").unwrap();
    assert_that(cargo_process("new").arg("foo").env("USER", Some("foo"))
                                    .cwd(td.path().clone()),
                execs().with_status(0));

    let toml = td.path().join("foo/Cargo.toml");
    let toml = File::open(&toml).read_to_string().unwrap();
    assert!(toml.as_slice().contains(r#"authors = ["foo"]"#));
});

test!(finds_author_username {
    // Use a temp dir to make sure we don't pick up .cargo/config somewhere in
    // the hierarchy
    let td = TempDir::new("cargo").unwrap();
    assert_that(cargo_process("new").arg("foo")
                                    .env("USER", None::<&str>)
                                    .env("USERNAME", Some("foo"))
                                    .cwd(td.path().clone()),
                execs().with_status(0));

    let toml = td.path().join("foo/Cargo.toml");
    let toml = File::open(&toml).read_to_string().unwrap();
    assert!(toml.as_slice().contains(r#"authors = ["foo"]"#));
});

test!(finds_author_git {
    my_process("git").args(&["config", "--global", "user.name", "bar"])
                     .exec().unwrap();
    my_process("git").args(&["config", "--global", "user.email", "baz"])
                     .exec().unwrap();
    assert_that(cargo_process("new").arg("foo").env("USER", Some("foo")),
                execs().with_status(0));

    let toml = paths::root().join("foo/Cargo.toml");
    let toml = File::open(&toml).read_to_string().unwrap();
    assert!(toml.as_slice().contains(r#"authors = ["bar <baz>"]"#));
});

test!(author_prefers_cargo {
    my_process("git").args(&["config", "--global", "user.name", "bar"])
                     .exec().unwrap();
    my_process("git").args(&["config", "--global", "user.email", "baz"])
                     .exec().unwrap();
    let root = paths::root();
    fs::mkdir(&root.join(".cargo"), USER_RWX).unwrap();
    File::create(&root.join(".cargo/config")).write_str(r#"
        [cargo-new]
        name = "new-foo"
        email = "new-bar"
        git = false
    "#).unwrap();

    assert_that(cargo_process("new").arg("foo").env("USER", Some("foo")),
                execs().with_status(0));

    let toml = paths::root().join("foo/Cargo.toml");
    let toml = File::open(&toml).read_to_string().unwrap();
    assert!(toml.as_slice().contains(r#"authors = ["new-foo <new-bar>"]"#));
    assert!(!root.join("foo/.gitignore").exists());
});

test!(git_prefers_command_line {
    let root = paths::root();
    let td = TempDir::new("cargo").unwrap();
    fs::mkdir(&root.join(".cargo"), USER_RWX).unwrap();
    File::create(&root.join(".cargo/config")).write_str(r#"
        [cargo-new]
        vcs = "none"
        name = "foo"
        email = "bar"
    "#).unwrap();

    assert_that(cargo_process("new").arg("foo").arg("--vcs").arg("git").cwd(td.path().clone())
                                    .env("USER", Some("foo")),
                execs().with_status(0));
    assert!(td.path().join("foo/.gitignore").exists());
});

test!(subpackage_no_git {
    assert_that(cargo_process("new").arg("foo").env("USER", Some("foo")),
                execs().with_status(0));

    let subpackage = paths::root().join("foo").join("components");
    fs::mkdir(&subpackage, USER_RWX).unwrap();
    assert_that(cargo_process("new").arg("foo/components/subcomponent")
                                    .env("USER", Some("foo")),
                execs().with_status(0));

    assert_that(&paths::root().join("foo/components/subcomponent/.git"),
                 is_not(existing_file()));
    assert_that(&paths::root().join("foo/components/subcomponent/.gitignore"),
                 is_not(existing_file()));
});

test!(unknown_flags {
    assert_that(cargo_process("new").arg("foo").arg("--flag"),
                execs().with_status(1)
                       .with_stderr("\
Unknown flag: '--flag'

Usage:
    cargo new [..]
    cargo new [..]
"));
});
