use std::fs::File;

use git2;

use support::git;
use support::{basic_manifest, execs, project};
use support::{is_nightly, ChannelChanger};
use support::hamcrest::assert_that;

#[test]
fn do_not_fix_broken_builds() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                pub fn foo() {
                    let mut x = 3;
                    drop(x);
                }

                pub fn foo2() {
                    let _x: u32 = "a";
                }
            "#,
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(101),
    );
    assert!(p.read_file("src/lib.rs").contains("let mut x = 3;"));
}

#[test]
fn fix_broken_if_requested() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                fn foo(a: &u32) -> u32 { a + 1 }
                pub fn bar() {
                    foo(1);
                }
            "#,
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs --broken-code")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0),
    );
}

#[test]
fn broken_fixes_backed_out() {
    let p = project()
        .file(
            "foo/Cargo.toml",
            r#"
                [package]
                name = 'foo'
                version = '0.1.0'
                [workspace]
            "#,
        )
        .file(
            "foo/src/main.rs",
            r##"
                use std::env;
                use std::fs;
                use std::io::Write;
                use std::path::{Path, PathBuf};
                use std::process::{self, Command};

                fn main() {
                    let is_lib_rs = env::args_os()
                        .map(PathBuf::from)
                        .any(|l| l == Path::new("src/lib.rs"));
                    if is_lib_rs {
                        let path = PathBuf::from(env::var_os("OUT_DIR").unwrap());
                        let path = path.join("foo");
                        if path.exists() {
                            fs::File::create("src/lib.rs")
                                .unwrap()
                                .write_all(b"not rust code")
                                .unwrap();
                        } else {
                            fs::File::create(&path).unwrap();
                        }
                    }

                    let status = Command::new("rustc")
                        .args(env::args().skip(1))
                        .status()
                        .expect("failed to run rustc");
                    process::exit(status.code().unwrap_or(2));
                }
            "##,
        )
        .file(
            "bar/Cargo.toml",
            r#"
                [package]
                name = 'bar'
                version = '0.1.0'
                [workspace]
            "#,
        )
        .file("bar/build.rs", "fn main() {}")
        .file(
            "bar/src/lib.rs",
            r#"
                pub fn foo() {
                    let mut x = 3;
                    drop(x);
                }
            "#,
        )
        .build();

    // Build our rustc shim
    assert_that(
        p.cargo("build").cwd(p.root().join("foo")),
        execs().with_status(0),
    );

    // Attempt to fix code, but our shim will always fail the second compile
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .cwd(p.root().join("bar"))
            .env("__CARGO_FIX_YOLO", "1")
            .env("RUSTC", p.root().join("foo/target/debug/foo")),
        execs()
            .with_status(101)
            .with_stderr_contains("[..]not rust code[..]")
            .with_stderr_contains("\
            warning: failed to automatically apply fixes suggested by rustc \
            to crate `bar`\n\
            \n\
            after fixes were automatically applied the compiler reported \
            errors within these files:\n\
            \n  \
            * src[/]lib.rs\n\
            \n\
            This likely indicates a bug in either rustc or cargo itself,\n\
            and we would appreciate a bug report! You're likely to see \n\
            a number of compiler warnings after this message which cargo\n\
            attempted to fix but failed. If you could open an issue at\n\
            https://github.com/rust-lang/cargo/issues\n\
            quoting the full output of this command we'd be very appreciative!\
            ")
            .with_stderr_does_not_contain("[..][FIXING][..]"),
    );
}

#[test]
fn fix_path_deps() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.1.0"

                [dependencies]
                bar = { path = 'bar' }

                [workspace]
            "#,
        )
        .file(
            "src/lib.rs",
            r#"
                extern crate bar;

                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }
            "#,
        )
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file(
            "bar/src/lib.rs",
            r#"
                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }
            "#,
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs -p foo -p bar")
            .env("__CARGO_FIX_YOLO", "1"),
        execs()
            .with_status(0)
            .with_stdout("")
            .with_stderr("\
[CHECKING] bar v0.1.0 ([..])
[FIXING] bar[/]src[/]lib.rs (1 fix)
[CHECKING] foo v0.1.0 ([..])
[FIXING] src[/]lib.rs (1 fix)
[FINISHED] [..]
")
    );
}

#[test]
fn do_not_fix_non_relevant_deps() {
    let p = project()
        .no_manifest()
        .file(
            "foo/Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.1.0"

                [dependencies]
                bar = { path = '../bar' }

                [workspace]
            "#,
        )
        .file("foo/src/lib.rs", "")
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file(
            "bar/src/lib.rs",
            r#"
                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }
            "#,
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1")
            .cwd(p.root().join("foo")),
        execs().with_status(0)
    );

    assert!(p.read_file("bar/src/lib.rs").contains("mut"));
}

#[test]
fn prepare_for_2018() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                #![allow(unused)]
                #![feature(rust_2018_preview)]

                mod foo {
                    pub const FOO: &str = "fooo";
                }

                mod bar {
                    use ::foo::FOO;
                }

                fn main() {
                    let x = ::foo::FOO;
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (2 fixes)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );

    println!("{}", p.read_file("src/lib.rs"));
    assert!(p.read_file("src/lib.rs").contains("use crate::foo::FOO;"));
    assert!(p.read_file("src/lib.rs").contains("let x = crate::foo::FOO;"));
}

#[test]
fn local_paths() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                #![feature(rust_2018_preview)]

                use test::foo;

                mod test {
                    pub fn foo() {}
                }

                pub fn f() {
                    foo();
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (1 fix)
[FINISHED] [..]
";

    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );

    println!("{}", p.read_file("src/lib.rs"));
    assert!(p.read_file("src/lib.rs").contains("use crate::test::foo;"));
}

#[test]
fn local_paths_no_fix() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                use test::foo;

                mod test {
                    pub fn foo() {}
                }

                pub fn f() {
                    foo();
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
warning: failed to find `#![feature(rust_2018_preview)]` in `src[/]lib.rs`
this may cause `cargo fix` to not be able to fix all
issues in preparation for the 2018 edition
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn upgrade_extern_crate() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                cargo-features = ["edition"]

                [package]
                name = "foo"
                version = "0.1.0"
                edition = '2018'

                [workspace]

                [dependencies]
                bar = { path = 'bar' }
            "#,
        )
        .file(
            "src/lib.rs",
            r#"
                #![warn(rust_2018_idioms)]
                extern crate bar;

                use bar::bar;

                pub fn foo() {
                    ::bar::bar();
                    bar();
                }
            "#,
        )
        .file("bar/Cargo.toml", &basic_manifest("bar", "0.1.0"))
        .file("bar/src/lib.rs", "pub fn bar() {}")
        .build();

    let stderr = "\
[CHECKING] bar v0.1.0 ([..])
[CHECKING] foo v0.1.0 ([..])
[FIXING] src[/]lib.rs (1 fix)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1")
            .masquerade_as_nightly_cargo(),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
    println!("{}", p.read_file("src/lib.rs"));
    assert!(!p.read_file("src/lib.rs").contains("extern crate"));
}

#[test]
fn specify_rustflags() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                #![allow(unused)]
                #![feature(rust_2018_preview)]

                mod foo {
                    pub const FOO: &str = "fooo";
                }

                fn main() {
                    let x = ::foo::FOO;
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (1 fix)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs")
            .env("RUSTFLAGS", "-C target-cpu=native"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn no_changes_necessary() {
    let p = project()
        .file("src/lib.rs", "")
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --allow-no-vcs"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn fixes_extra_mut() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (1 fix)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn fixes_two_missing_ampersands() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                pub fn foo() -> u32 {
                    let mut x = 3;
                    let mut y = 3;
                    x + y
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (2 fixes)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn tricky() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                pub fn foo() -> u32 {
                    let mut x = 3; let mut y = 3;
                    x + y
                }
            "#,
        )
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
[FIXING] src[/]lib.rs (2 fixes)
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0).with_stderr(stderr).with_stdout(""),
    );
}

#[test]
fn preserve_line_endings() {
    let p = project()
        .file(
            "src/lib.rs",
            "\
             fn add(a: &u32) -> u32 { a + 1 }\r\n\
             pub fn foo() -> u32 { let mut x = 3; add(&x) }\r\n\
             ",
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0),
    );
    assert!(p.read_file("src/lib.rs").contains("\r\n"));
}

#[test]
fn fix_deny_warnings() {
    let p = project()
        .file(
            "src/lib.rs",
            "\
                #![deny(warnings)]
                pub fn foo() { let mut x = 3; drop(x); }
            ",
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0),
    );
}

#[test]
fn fix_deny_warnings_but_not_others() {
    let p = project()
        .file(
            "src/lib.rs",
            "
                #![deny(warnings)]

                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }

                fn bar() {}
            ",
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0),
    );
    assert!(!p.read_file("src/lib.rs").contains("let mut x = 3;"));
    assert!(p.read_file("src/lib.rs").contains("fn bar() {}"));
}

#[test]
fn fix_two_files() {
    let p = project()
        .file(
            "src/lib.rs",
            "
                pub mod bar;

                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }
            ",
        )
        .file(
            "src/bar.rs",
            "
                pub fn foo() -> u32 {
                    let mut x = 3;
                    x
                }

            ",
        )
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs()
            .with_status(0)
            .with_stderr_contains("[FIXING] src[/]bar.rs (1 fix)")
            .with_stderr_contains("[FIXING] src[/]lib.rs (1 fix)"),
    );
    assert!(!p.read_file("src/lib.rs").contains("let mut x = 3;"));
    assert!(!p.read_file("src/bar.rs").contains("let mut x = 3;"));
}

#[test]
fn fixes_missing_ampersand() {
    let p = project()
        .file("src/main.rs", "fn main() { let mut x = 3; drop(x); }")
        .file(
            "src/lib.rs",
            r#"
                pub fn foo() { let mut x = 3; drop(x); }

                #[test]
                pub fn foo2() { let mut x = 3; drop(x); }
            "#,
        )
        .file(
            "tests/a.rs",
            r#"
                #[test]
                pub fn foo() { let mut x = 3; drop(x); }
            "#,
        )
        .file("examples/foo.rs", "fn main() { let mut x = 3; drop(x); }")
        .file("build.rs", "fn main() { let mut x = 3; drop(x); }")
        .build();

    assert_that(
        p.cargo("fix --all-targets --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs()
            .with_status(0)
            .with_stdout("")
            .with_stderr_contains("[COMPILING] foo v0.0.1 ([..])")
            .with_stderr_contains("[FIXING] build.rs (1 fix)")
            // Don't assert number of fixes for this one, as we don't know if we're
            // fixing it once or twice! We run this all concurrently, and if we
            // compile (and fix) in `--test` mode first, we get two fixes. Otherwise
            // we'll fix one non-test thing, and then fix another one later in
            // test mode.
            .with_stderr_contains("[FIXING] src[/]lib.rs[..]")
            .with_stderr_contains("[FIXING] src[/]main.rs (1 fix)")
            .with_stderr_contains("[FIXING] examples[/]foo.rs (1 fix)")
            .with_stderr_contains("[FIXING] tests[/]a.rs (1 fix)")
            .with_stderr_contains("[FINISHED] [..]"),
    );
    assert_that(p.cargo("build"), execs().with_status(0));
    assert_that(p.cargo("test"), execs().with_status(0));
}

#[test]
fn fix_features() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.1.0"

                [features]
                bar = []

                [workspace]
            "#,
        )
        .file(
            "src/lib.rs",
            r#"
            #[cfg(feature = "bar")]
            pub fn foo() -> u32 { let mut x = 3; x }
        "#,
        )
        .build();

    assert_that(p.cargo("fix --allow-no-vcs"), execs().with_status(0));
    assert_that(p.cargo("build"), execs().with_status(0));
    assert_that(p.cargo("fix --features bar --allow-no-vcs"), execs().with_status(0));
    assert_that(p.cargo("build --features bar"), execs().with_status(0));
}

#[test]
fn shows_warnings() {
    let p = project()
        .file("src/lib.rs", "use std::default::Default; pub fn foo() {}")
        .build();

    assert_that(
        p.cargo("fix --allow-no-vcs"),
        execs().with_status(0).with_stderr_contains("[..]warning: unused import[..]"),
    );
}

#[test]
fn warns_if_no_vcs_detected() {
    let p = project()
        .file("src/lib.rs", "pub fn foo() {}")
        .build();

    assert_that(
        p.cargo("fix"),
        execs()
            .with_status(101)
            .with_stderr("\
error: no VCS found for this project and `cargo fix` can potentially perform \
destructive changes; if you'd like to suppress this error pass `--allow-no-vcs`\
")
    );
    assert_that(
        p.cargo("fix --allow-no-vcs"),
        execs().with_status(0),
    );
}

#[test]
fn warns_about_dirty_working_directory() {
    let p = project()
        .file("src/lib.rs", "pub fn foo() {}")
        .build();

    let repo = git2::Repository::init(&p.root()).unwrap();
    let mut cfg = t!(repo.config());
    t!(cfg.set_str("user.email", "foo@bar.com"));
    t!(cfg.set_str("user.name", "Foo Bar"));
    drop(cfg);
    git::add(&repo);
    git::commit(&repo);
    File::create(p.root().join("src/lib.rs")).unwrap();

    assert_that(
        p.cargo("fix"),
        execs()
            .with_status(101)
            .with_stderr("\
error: the working directory of this project is detected as dirty, and `cargo \
fix` can potentially perform destructive changes; if you'd like to \
suppress this error pass `--allow-dirty`, or commit the changes to \
these files:

  * src/lib.rs


")
    );
    assert_that(
        p.cargo("fix --allow-dirty"),
        execs().with_status(0),
    );
}

#[test]
fn does_not_warn_about_clean_working_directory() {
    let p = project()
        .file("src/lib.rs", "pub fn foo() {}")
        .build();

    let repo = git2::Repository::init(&p.root()).unwrap();
    let mut cfg = t!(repo.config());
    t!(cfg.set_str("user.email", "foo@bar.com"));
    t!(cfg.set_str("user.name", "Foo Bar"));
    drop(cfg);
    git::add(&repo);
    git::commit(&repo);

    assert_that(
        p.cargo("fix"),
        execs().with_status(0),
    );
}

#[test]
fn does_not_warn_about_dirty_ignored_files() {
    let p = project()
        .file("src/lib.rs", "pub fn foo() {}")
        .file(".gitignore", "bar\n")
        .build();

    let repo = git2::Repository::init(&p.root()).unwrap();
    let mut cfg = t!(repo.config());
    t!(cfg.set_str("user.email", "foo@bar.com"));
    t!(cfg.set_str("user.name", "Foo Bar"));
    drop(cfg);
    git::add(&repo);
    git::commit(&repo);
    File::create(p.root().join("bar")).unwrap();

    assert_that(
        p.cargo("fix"),
        execs().with_status(0),
    );
}

#[test]
fn fix_all_targets_by_default() {
    let p = project()
        .file("src/lib.rs", "pub fn foo() { let mut x = 3; drop(x); }")
        .file("tests/foo.rs", "pub fn foo() { let mut x = 3; drop(x); }")
        .build();
    assert_that(
        p.cargo("fix --allow-no-vcs")
            .env("__CARGO_FIX_YOLO", "1"),
        execs().with_status(0),
    );
    assert!(!p.read_file("src/lib.rs").contains("let mut x"));
    assert!(!p.read_file("tests/foo.rs").contains("let mut x"));
}

#[test]
fn prepare_for_and_enable() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                cargo-features = ['edition']

                [package]
                name = 'foo'
                version = '0.1.0'
                edition = '2018'
            "#,
        )
        .file("src/lib.rs", "")
        .build();

    let stderr = "\
[CHECKING] foo v0.1.0 ([..])
error: cannot prepare for the 2018 edition when it is enabled, so cargo cannot
automatically fix errors in `src[/]lib.rs`

To prepare for the 2018 edition you should first remove `edition = '2018'` from
your `Cargo.toml` and then rerun this command. Once all warnings have been fixed
then you can re-enable the `edition` key in `Cargo.toml`. For some more
information about transitioning to the 2018 edition see:

  https://[..]

";
    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs")
            .masquerade_as_nightly_cargo(),
        execs()
            .with_stderr_contains(stderr)
            .with_status(101),
    );
}

#[test]
fn prepare_for_without_feature_issues_warning() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file("src/lib.rs", "")
        .build();

    let stderr = "\
[CHECKING] foo v0.0.1 ([..])
warning: failed to find `#![feature(rust_2018_preview)]` in `src[/]lib.rs`
this may cause `cargo fix` to not be able to fix all
issues in preparation for the 2018 edition
[FINISHED] [..]
";
    assert_that(
        p.cargo("fix --prepare-for 2018 --allow-no-vcs")
            .masquerade_as_nightly_cargo(),
        execs()
            .with_stderr(stderr)
            .with_status(0),
    );
}

#[test]
fn fix_overlapping() {
    if !is_nightly() {
        return
    }
    let p = project()
        .file(
            "src/lib.rs",
            r#"
                #![feature(rust_2018_preview)]

                pub fn foo<T>() {}
                pub struct A;

                pub mod bar {
                    pub fn baz() {
                        ::foo::<::A>();
                    }
                }
            "#
        )
        .build();

    let stderr = "\
[CHECKING] foo [..]
[FIXING] src[/]lib.rs (2 fixes)
[FINISHED] dev [..]
";

    assert_that(
        p.cargo("fix --allow-no-vcs --prepare-for 2018 --lib"),
        execs().with_status(0).with_stderr(stderr),
    );

    let contents = p.read_file("src/lib.rs");
    println!("{}", contents);
    assert!(contents.contains("crate::foo::<crate::A>()"));
}
