use std::fs::File;
use std::io::prelude::*;
use std::str;

use support::{project, execs, basic_bin_manifest, basic_lib_manifest};
use support::{DOCTEST};
use support::paths::CargoPathExt;
use hamcrest::{assert_that, existing_file, is_not};
use cargo::util::process;

fn setup() {}

test!(cargo_test_simple {
    let p = project("foo")
        .file("Cargo.toml", &basic_bin_manifest("foo"))
        .file("src/foo.rs", r#"
            fn hello() -> &'static str {
                "hello"
            }

            pub fn main() {
                println!("{}", hello())
            }

            #[test]
            fn test_hello() {
                assert_eq!(hello(), "hello")
            }"#);

    assert_that(p.cargo_process("build"), execs());
    assert_that(&p.bin("foo"), existing_file());

    assert_that(process(&p.bin("foo")),
                execs().with_stdout("hello\n"));

    assert_that(p.cargo("test"),
                execs().with_stdout(format!("\
[COMPILING] foo v0.5.0 ({})
[RUNNING] target[..]foo-[..]

running 1 test
test test_hello ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", p.url())));
});

test!(cargo_test_release {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            authors = []
            version = "0.1.0"

            [dependencies]
            bar = { path = "bar" }
        "#)
        .file("src/lib.rs", r#"
            extern crate bar;
            pub fn foo() { bar::bar(); }

            #[test]
            fn test() { foo(); }
        "#)
        .file("tests/test.rs", r#"
            extern crate foo;

            #[test]
            fn test() { foo::foo(); }
        "#)
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []
        "#)
        .file("bar/src/lib.rs", "pub fn bar() {}");

    assert_that(p.cargo_process("test").arg("-v").arg("--release"),
                execs().with_stdout(format!("\
[COMPILING] bar v0.0.1 ({dir}/bar)
[RUNNING] [..] -C opt-level=3 [..]
[COMPILING] foo v0.1.0 ({dir})
[RUNNING] [..] -C opt-level=3 [..]
[RUNNING] [..] -C opt-level=3 [..]
[RUNNING] [..] -C opt-level=3 [..]
[RUNNING] `[..]target[..]foo-[..]`

running 1 test
test test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] `[..]target[..]test-[..]`

running 1 test
test test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo
[RUNNING] `rustdoc --test [..]lib.rs[..]`

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

", dir = p.url(), doctest = DOCTEST)));
});

test!(cargo_test_verbose {
    let p = project("foo")
        .file("Cargo.toml", &basic_bin_manifest("foo"))
        .file("src/foo.rs", r#"
            fn main() {}
            #[test] fn test_hello() {}
        "#);

    assert_that(p.cargo_process("test").arg("-v").arg("hello"),
        execs().with_stdout(format!("\
[COMPILING] foo v0.5.0 ({url})
[RUNNING] `rustc src[..]foo.rs [..]`
[RUNNING] `[..]target[..]foo-[..] hello`

running 1 test
test test_hello ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", url = p.url())));
});

test!(many_similar_names {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
            #[test] fn lib_test() {}
        ")
        .file("src/main.rs", "
            extern crate foo;
            fn main() {}
            #[test] fn bin_test() { foo::foo() }
        ")
        .file("tests/foo.rs", r#"
            extern crate foo;
            #[test] fn test_test() { foo::foo() }
        "#);

    let output = p.cargo_process("test").arg("-v").exec_with_output().unwrap();
    let output = str::from_utf8(&output.stdout).unwrap();
    assert!(output.contains("test bin_test"), "bin_test missing\n{}", output);
    assert!(output.contains("test lib_test"), "lib_test missing\n{}", output);
    assert!(output.contains("test test_test"), "test_test missing\n{}", output);
});

test!(cargo_test_failing_test {
    let p = project("foo")
        .file("Cargo.toml", &basic_bin_manifest("foo"))
        .file("src/foo.rs", r#"
            fn hello() -> &'static str {
                "hello"
            }

            pub fn main() {
                println!("{}", hello())
            }

            #[test]
            fn test_hello() {
                assert_eq!(hello(), "nope")
            }"#);

    assert_that(p.cargo_process("build"), execs());
    assert_that(&p.bin("foo"), existing_file());

    assert_that(process(&p.bin("foo")),
                execs().with_stdout("hello\n"));

    assert_that(p.cargo("test"),
                execs().with_stdout_contains(format!("\
[COMPILING] foo v0.5.0 ({url})
[RUNNING] target[..]foo-[..]

running 1 test
test test_hello ... FAILED

failures:

---- test_hello stdout ----
<tab>thread 'test_hello' panicked at 'assertion failed: \
    `(left == right)` (left: \
    `\"hello\"`, right: `\"nope\"`)', src[..]foo.rs:12
", url = p.url()))
                    .with_stdout_contains("\
failures:
    test_hello

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured
")
                    .with_status(101));
});

test!(test_with_lib_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[bin]]
            name = "baz"
            path = "src/main.rs"
        "#)
        .file("src/lib.rs", r#"
            ///
            /// ```rust
            /// extern crate foo;
            /// fn main() {
            ///     println!("{:?}", foo::foo());
            /// }
            /// ```
            ///
            pub fn foo(){}
            #[test] fn lib_test() {}
        "#)
        .file("src/main.rs", "
            extern crate foo;

            fn main() {}

            #[test]
            fn bin_test() {}
        ");

    assert_that(p.cargo_process("test"),
        execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ({})
[RUNNING] target[..]baz-[..]

running 1 test
test bin_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]foo[..]

running 1 test
test lib_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 1 test
test foo_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
        p.url(), doctest = DOCTEST)))
});

test!(test_with_deep_lib_dep {
    let p = project("bar")
        .file("Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []

            [dependencies.foo]
            path = "../foo"
        "#)
        .file("src/lib.rs", "
            extern crate foo;
            /// ```
            /// bar::bar();
            /// ```
            pub fn bar() {}

            #[test]
            fn bar_test() {
                foo::foo();
            }
        ");
    let p2 = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}

            #[test]
            fn foo_test() {}
        ");

    p2.build();
    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ([..])
[COMPILING] bar v0.0.1 ({dir})
[RUNNING] target[..]

running 1 test
test bar_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} bar

running 1 test
test bar_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST,
                       dir = p.url())));
});

test!(external_test_explicit {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[test]]
            name = "test"
            path = "src/test.rs"
        "#)
        .file("src/lib.rs", r#"
            pub fn get_hello() -> &'static str { "Hello" }

            #[test]
            fn internal_test() {}
        "#)
        .file("src/test.rs", r#"
            extern crate foo;

            #[test]
            fn external_test() { assert_eq!(foo::get_hello(), "Hello") }
        "#);

    assert_that(p.cargo_process("test"),
        execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ({})
[RUNNING] target[..]foo-[..]

running 1 test
test internal_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]test-[..]

running 1 test
test external_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
        p.url(), doctest = DOCTEST)))
});

test!(external_test_implicit {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
            pub fn get_hello() -> &'static str { "Hello" }

            #[test]
            fn internal_test() {}
        "#)
        .file("tests/external.rs", r#"
            extern crate foo;

            #[test]
            fn external_test() { assert_eq!(foo::get_hello(), "Hello") }
        "#);

    assert_that(p.cargo_process("test"),
        execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ({})
[RUNNING] target[..]external-[..]

running 1 test
test external_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]foo-[..]

running 1 test
test internal_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

", p.url(), doctest = DOCTEST)))
});

test!(dont_run_examples {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
        "#)
        .file("examples/dont-run-me-i-will-fail.rs", r#"
            fn main() { panic!("Examples should not be run by 'cargo test'"); }
        "#);
    assert_that(p.cargo_process("test"),
                execs().with_status(0));
});

test!(pass_through_command_line {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            #[test] fn foo() {}
            #[test] fn bar() {}
        ");

    assert_that(p.cargo_process("test").arg("bar"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]foo-[..]

running 1 test
test bar ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST,
                       dir = p.url())));

    assert_that(p.cargo("test").arg("foo"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST)));
});

// Regression test for running cargo-test twice with
// tests in an rlib
test!(cargo_test_twice {
    let p = project("test_twice")
        .file("Cargo.toml", &basic_lib_manifest("test_twice"))
        .file("src/test_twice.rs", r#"
            #![crate_type = "rlib"]

            #[test]
            fn dummy_test() { }
            "#);

    p.cargo_process("build");

    for _ in 0..2 {
        assert_that(p.cargo("test"),
                    execs().with_status(0));
    }
});

test!(lib_bin_same_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            name = "foo"
            [[bin]]
            name = "foo"
        "#)
        .file("src/lib.rs", "
            #[test] fn lib_test() {}
        ")
        .file("src/main.rs", "
            extern crate foo;

            #[test]
            fn bin_test() {}
        ");

    assert_that(p.cargo_process("test"),
        execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ({})
[RUNNING] target[..]foo-[..]

running 1 test
test [..] ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]foo-[..]

running 1 test
test [..] ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
        p.url(), doctest = DOCTEST)))
});

test!(lib_with_standard_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            /// ```
            /// syntax::foo();
            /// ```
            pub fn foo() {}

            #[test]
            fn foo_test() {}
        ")
        .file("tests/test.rs", "
            extern crate syntax;

            #[test]
            fn test() { syntax::foo() }
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] syntax v0.0.1 ({dir})
[RUNNING] target[..]syntax-[..]

running 1 test
test foo_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]test-[..]

running 1 test
test test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} syntax

running 1 test
test foo_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST, dir = p.url())));
});

test!(lib_with_standard_name2 {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            name = "syntax"
            test = false
            doctest = false
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
        ")
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] syntax v0.0.1 ({dir})
[RUNNING] target[..]syntax-[..]

running 1 test
test test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
                       dir = p.url())));
});

test!(lib_without_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            test = false
            doctest = false
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
        ")
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] syntax v0.0.1 ({dir})
[RUNNING] target[..]syntax-[..]

running 1 test
test test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", dir = p.url())));
});

test!(bin_without_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            test = false
            doctest = false

            [[bin]]
            path = "src/main.rs"
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
        ")
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(101)
                       .with_stderr(&format!("\
[ERROR] failed to parse manifest at `[..]`

Caused by:
  binary target bin.name is required")));
});

test!(bench_without_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            test = false
            doctest = false

            [[bench]]
            path = "src/bench.rs"
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
        ")
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ")
        .file("src/bench.rs", "
            #![feature(test)]
            extern crate syntax;
            extern crate test;

            #[bench]
            fn external_bench(_b: &mut test::Bencher) {}
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(101)
                       .with_stderr(&format!("\
[ERROR] failed to parse manifest at `[..]`

Caused by:
  bench target bench.name is required")));
});

test!(test_without_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            test = false
            doctest = false

            [[test]]
            path = "src/test.rs"
        "#)
        .file("src/lib.rs", r#"
            pub fn foo() {}
            pub fn get_hello() -> &'static str { "Hello" }
        "#)
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ")
        .file("src/test.rs", r#"
            extern crate syntax;

            #[test]
            fn external_test() { assert_eq!(syntax::get_hello(), "Hello") }
        "#);

    assert_that(p.cargo_process("test"),
                execs().with_status(101)
                       .with_stderr(&format!("\
[ERROR] failed to parse manifest at `[..]`

Caused by:
  test target test.name is required")));
});

test!(example_without_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "syntax"
            version = "0.0.1"
            authors = []

            [lib]
            test = false
            doctest = false

            [[example]]
            path = "examples/example.rs"
        "#)
        .file("src/lib.rs", "
            pub fn foo() {}
        ")
        .file("src/main.rs", "
            extern crate syntax;

            fn main() {}

            #[test]
            fn test() { syntax::foo() }
        ")
        .file("examples/example.rs", r#"
            extern crate syntax;

            fn main() {
                println!("example1");
            }
        "#);

    assert_that(p.cargo_process("test"),
                execs().with_status(101)
                       .with_stderr(&format!("\
[ERROR] failed to parse manifest at `[..]`

Caused by:
  example target example.name is required")));
});

test!(bin_there_for_integration {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", "
            fn main() { std::process::exit(101); }
            #[test] fn main_test() {}
        ")
        .file("tests/foo.rs", r#"
            use std::process::Command;
            #[test]
            fn test_test() {
                let status = Command::new("target/debug/foo").status().unwrap();
                assert_eq!(status.code(), Some(101));
            }
        "#);

    let output = p.cargo_process("test").arg("-v").exec_with_output().unwrap();
    let output = str::from_utf8(&output.stdout).unwrap();
    assert!(output.contains("main_test ... ok"), "no main_test\n{}", output);
    assert!(output.contains("test_test ... ok"), "no test_test\n{}", output);
});

test!(test_dylib {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            name = "foo"
            crate_type = ["dylib"]

            [dependencies.bar]
            path = "bar"
        "#)
        .file("src/lib.rs", r#"
            extern crate bar as the_bar;

            pub fn bar() { the_bar::baz(); }

            #[test]
            fn foo() { bar(); }
        "#)
        .file("tests/test.rs", r#"
            extern crate foo as the_foo;

            #[test]
            fn foo() { the_foo::bar(); }
        "#)
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []

            [lib]
            name = "bar"
            crate_type = ["dylib"]
        "#)
        .file("bar/src/lib.rs", "
             pub fn baz() {}
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] bar v0.0.1 ({dir}/bar)
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]test-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
                       dir = p.url())));
    p.root().move_into_the_past().unwrap();
    assert_that(p.cargo("test"),
                execs().with_status(0)
                       .with_stdout(format!("\
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]test-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

")));

});

test!(test_twice_with_build_cmd {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
            build = "build.rs"
        "#)
        .file("build.rs", "fn main() {}")
        .file("src/lib.rs", "
            #[test]
            fn foo() {}
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST,
                       dir = p.url())));

    assert_that(p.cargo("test"),
                execs().with_status(0)
                       .with_stdout(format!("\
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST)));
});

test!(test_then_build {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            #[test]
            fn foo() {}
        ");

    assert_that(p.cargo_process("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]foo-[..]

running 1 test
test foo ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
                       doctest = DOCTEST,
                       dir = p.url())));

    assert_that(p.cargo("build"),
                execs().with_status(0)
                       .with_stdout(""));
});

test!(test_no_run {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "
            #[test]
            fn foo() { panic!() }
        ");

    assert_that(p.cargo_process("test").arg("--no-run"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
",
                       dir = p.url())));
});

test!(test_run_specific_bin_target {
    let prj = project("foo")
        .file("Cargo.toml" , r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[bin]]
            name="bin1"
            path="src/bin1.rs"

            [[bin]]
            name="bin2"
            path="src/bin2.rs"
        "#)
        .file("src/bin1.rs", "#[test] fn test1() { }")
        .file("src/bin2.rs", "#[test] fn test2() { }");

    let expected_stdout = format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]bin2-[..]

running 1 test
test test2 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
       dir = prj.url());

    assert_that(prj.cargo_process("test").arg("--bin").arg("bin2"),
        execs().with_status(0).with_stdout(&expected_stdout));
});

test!(test_run_specific_test_target {
    let prj = project("foo")
        .file("Cargo.toml" , r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/bin/a.rs", "fn main() { }")
        .file("src/bin/b.rs", "#[test] fn test_b() { } fn main() { }")
        .file("tests/a.rs", "#[test] fn test_a() { }")
        .file("tests/b.rs", "#[test] fn test_b() { }");

    let expected_stdout = format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]b-[..]

running 1 test
test test_b ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

",
       dir = prj.url());

    assert_that(prj.cargo_process("test").arg("--test").arg("b"),
        execs().with_status(0).with_stdout(&expected_stdout));
});

test!(test_no_harness {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[bin]]
            name = "foo"
            test = false

            [[test]]
            name = "bar"
            path = "foo.rs"
            harness = false
        "#)
        .file("src/main.rs", "fn main() {}")
        .file("foo.rs", "fn main() {}");

    assert_that(p.cargo_process("test").arg("--").arg("--nocapture"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]bar-[..]
",
                       dir = p.url())));
});

test!(selective_testing {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.d1]
                path = "d1"
            [dependencies.d2]
                path = "d2"

            [lib]
                name = "foo"
                doctest = false
        "#)
        .file("src/lib.rs", "")
        .file("d1/Cargo.toml", r#"
            [package]
            name = "d1"
            version = "0.0.1"
            authors = []

            [lib]
                name = "d1"
                doctest = false
        "#)
        .file("d1/src/lib.rs", "")
        .file("d1/src/main.rs", "extern crate d1; fn main() {}")
        .file("d2/Cargo.toml", r#"
            [package]
            name = "d2"
            version = "0.0.1"
            authors = []

            [lib]
                name = "d2"
                doctest = false
        "#)
        .file("d2/src/lib.rs", "")
        .file("d2/src/main.rs", "extern crate d2; fn main() {}");
    p.build();

    println!("d1");
    assert_that(p.cargo("test").arg("-p").arg("d1"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] d1 v0.0.1 ({dir}/d1)
[RUNNING] target[..]d1-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]d1-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
   dir = p.url())));

    println!("d2");
    assert_that(p.cargo("test").arg("-p").arg("d2"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] d2 v0.0.1 ({dir}/d2)
[RUNNING] target[..]d2-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]d2-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
   dir = p.url())));

    println!("whole");
    assert_that(p.cargo("test"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] target[..]foo-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

",
   dir = p.url())));
});

test!(almost_cyclic_but_not_quite {
    let p = project("a")
        .file("Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []

            [dev-dependencies.b]
            path = "b"
            [dev-dependencies.c]
            path = "c"
        "#)
        .file("src/lib.rs", r#"
            #[cfg(test)] extern crate b;
            #[cfg(test)] extern crate c;
        "#)
        .file("b/Cargo.toml", r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []

            [dependencies.a]
            path = ".."
        "#)
        .file("b/src/lib.rs", r#"
            extern crate a;
        "#)
        .file("c/Cargo.toml", r#"
            [package]
            name = "c"
            version = "0.0.1"
            authors = []
        "#)
        .file("c/src/lib.rs", "");

    assert_that(p.cargo_process("build"), execs().with_status(0));
    assert_that(p.cargo("test"),
                execs().with_status(0));
});

test!(build_then_selective_test {
    let p = project("a")
        .file("Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []

            [dependencies.b]
            path = "b"
        "#)
        .file("src/lib.rs", "extern crate b;")
        .file("src/main.rs", "extern crate b; extern crate a; fn main() {}")
        .file("b/Cargo.toml", r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []
        "#)
        .file("b/src/lib.rs", "");

    assert_that(p.cargo_process("build"), execs().with_status(0));
    p.root().move_into_the_past().unwrap();
    assert_that(p.cargo("test").arg("-p").arg("b"),
                execs().with_status(0));
});

test!(example_dev_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dev-dependencies.bar]
            path = "bar"
        "#)
        .file("src/lib.rs", r#"
        "#)
        .file("examples/e1.rs", r#"
            extern crate bar;
            fn main() { }
        "#)
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []
        "#)
        .file("bar/src/lib.rs", r#"
            // make sure this file takes awhile to compile
            macro_rules! f0( () => (1) );
            macro_rules! f1( () => ({(f0!()) + (f0!())}) );
            macro_rules! f2( () => ({(f1!()) + (f1!())}) );
            macro_rules! f3( () => ({(f2!()) + (f2!())}) );
            macro_rules! f4( () => ({(f3!()) + (f3!())}) );
            macro_rules! f5( () => ({(f4!()) + (f4!())}) );
            macro_rules! f6( () => ({(f5!()) + (f5!())}) );
            macro_rules! f7( () => ({(f6!()) + (f6!())}) );
            macro_rules! f8( () => ({(f7!()) + (f7!())}) );
            pub fn bar() {
                f8!();
            }
        "#);
    assert_that(p.cargo_process("test"),
                execs().with_status(0));
    assert_that(p.cargo("run")
                 .arg("--example").arg("e1").arg("--release").arg("-v"),
                execs().with_status(0));
});

test!(selective_testing_with_docs {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.d1]
                path = "d1"
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// not valid rust
            /// ```
            pub fn foo() {}
        "#)
        .file("d1/Cargo.toml", r#"
            [package]
            name = "d1"
            version = "0.0.1"
            authors = []

            [lib]
            name = "d1"
            path = "d1.rs"
        "#)
        .file("d1/d1.rs", "");
    p.build();

    assert_that(p.cargo("test").arg("-p").arg("d1"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] d1 v0.0.1 ({dir}/d1)
[RUNNING] target[..]deps[..]d1[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

{doctest} d1

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

", dir = p.url(),
   doctest = DOCTEST)));
});

test!(example_bin_same_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/bin/foo.rs", r#"fn main() { println!("bin"); }"#)
        .file("examples/foo.rs", r#"fn main() { println!("example"); }"#);

    assert_that(p.cargo_process("test").arg("--no-run").arg("-v"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ({dir})
[RUNNING] `rustc [..]`
[RUNNING] `rustc [..]`
", dir = p.url())));

    assert_that(&p.bin("foo"), is_not(existing_file()));
    assert_that(&p.bin("examples/foo"), existing_file());

    assert_that(p.process(&p.bin("examples/foo")),
                execs().with_status(0).with_stdout("example\n"));

    assert_that(p.cargo("run"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ([..])
[RUNNING] [..]
bin
")));
    assert_that(&p.bin("foo"), existing_file());
});

test!(test_with_example_twice {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/bin/foo.rs", r#"fn main() { println!("bin"); }"#)
        .file("examples/foo.rs", r#"fn main() { println!("example"); }"#);

    println!("first");
    assert_that(p.cargo_process("test").arg("-v"),
                execs().with_status(0));
    assert_that(&p.bin("examples/foo"), existing_file());
    println!("second");
    assert_that(p.cargo("test").arg("-v"),
                execs().with_status(0));
    assert_that(&p.bin("examples/foo"), existing_file());
});

test!(example_with_dev_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            name = "foo"
            test = false
            doctest = false

            [dev-dependencies.a]
            path = "a"
        "#)
        .file("src/lib.rs", "")
        .file("examples/ex.rs", "extern crate a; fn main() {}")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#)
        .file("a/src/lib.rs", "");

    assert_that(p.cargo_process("test").arg("-v"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[..]
[..]
[..]
[..]
[RUNNING] `rustc [..] --crate-name ex [..] --extern a=[..]`
")));
});

test!(bin_is_preserved {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/main.rs", "fn main() {}");

    assert_that(p.cargo_process("build").arg("-v"),
                execs().with_status(0));
    assert_that(&p.bin("foo"), existing_file());

    println!("testing");
    assert_that(p.cargo("test").arg("-v"),
                execs().with_status(0));
    assert_that(&p.bin("foo"), existing_file());
});

test!(bad_example {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "");

    assert_that(p.cargo_process("run").arg("--example").arg("foo"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no example target named `foo`
")));
    assert_that(p.cargo_process("run").arg("--bin").arg("foo"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no bin target named `foo`
")));
});

test!(doctest_feature {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
            [features]
            bar = []
        "#)
        .file("src/lib.rs", r#"
            /// ```rust
            /// assert_eq!(foo::foo(), 1);
            /// ```
            #[cfg(feature = "bar")]
            pub fn foo() -> i32 { 1 }
        "#);

    assert_that(p.cargo_process("test").arg("--features").arg("bar"),
                execs().with_status(0).with_stdout(format!("\
[COMPILING] foo [..]
[RUNNING] target[..]foo[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 1 test
test foo_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", doctest = DOCTEST)))
});

test!(dashes_to_underscores {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo-bar"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// assert_eq!(foo_bar::foo(), 1);
            /// ```
            pub fn foo() -> i32 { 1 }
        "#);

    assert_that(p.cargo_process("test").arg("-v"),
                execs().with_status(0));
});

test!(doctest_dev_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dev-dependencies]
            b = { path = "b" }
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// extern crate b;
            /// ```
            pub fn foo() {}
        "#)
        .file("b/Cargo.toml", r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []
        "#)
        .file("b/src/lib.rs", "");

    assert_that(p.cargo_process("test").arg("-v"),
                execs().with_status(0));
});

test!(filter_no_doc_tests {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// extern crate b;
            /// ```
            pub fn foo() {}
        "#)
        .file("tests/foo.rs", "");

    assert_that(p.cargo_process("test").arg("--test=foo"),
                execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ([..])
[RUNNING] target[..]debug[..]foo[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

")));
});

test!(dylib_doctest {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            name = "foo"
            crate-type = ["rlib", "dylib"]
            test = false
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// foo::foo();
            /// ```
            pub fn foo() {}
        "#);

    assert_that(p.cargo_process("test"),
                execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ([..])
{doctest} foo

running 1 test
test foo_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", doctest = DOCTEST)));
});

test!(dylib_doctest2 {
    // can't doctest dylibs as they're statically linked together
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            name = "foo"
            crate-type = ["dylib"]
            test = false
        "#)
        .file("src/lib.rs", r#"
            /// ```
            /// foo::foo();
            /// ```
            pub fn foo() {}
        "#);

    assert_that(p.cargo_process("test"),
                execs().with_stdout(""));
});

test!(cyclic_dev_dep_doc_test {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dev-dependencies]
            bar = { path = "bar" }
        "#)
        .file("src/lib.rs", r#"
            //! ```
            //! extern crate bar;
            //! ```
        "#)
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []

            [dependencies]
            foo = { path = ".." }
        "#)
        .file("bar/src/lib.rs", r#"
            extern crate foo;
        "#);
    assert_that(p.cargo_process("test"),
                execs().with_stdout(format!("\
[COMPILING] foo v0.0.1 ([..])
[COMPILING] bar v0.0.1 ([..])
[RUNNING] target[..]foo[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 1 test
test _0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", doctest = DOCTEST)))
});

test!(dev_dep_with_build_script {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dev-dependencies]
            bar = { path = "bar" }
        "#)
        .file("src/lib.rs", "")
        .file("examples/foo.rs", "fn main() {}")
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []
            build = "build.rs"
        "#)
        .file("bar/src/lib.rs", "")
        .file("bar/build.rs", "fn main() {}");
    assert_that(p.cargo_process("test"),
                execs().with_status(0));
});

test!(no_fail_fast {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
        pub fn add_one(x: i32) -> i32{
            x + 1
        }

        /// ```rust
        /// use foo::sub_one;
        /// assert_eq!(sub_one(101), 100);
        /// ```
        pub fn sub_one(x: i32) -> i32{
            x - 1
        }
        "#)
        .file("tests/test_add_one.rs", r#"
        extern crate foo;
        use foo::*;

        #[test]
        fn add_one_test() {
            assert_eq!(add_one(1), 2);
        }

        #[test]
        fn fail_add_one_test() {
            assert_eq!(add_one(1), 1);
        }
        "#)
        .file("tests/test_sub_one.rs", r#"
        extern crate foo;
        use foo::*;

        #[test]
        fn sub_one_test() {
            assert_eq!(sub_one(1), 0);
        }
        "#);
    assert_that(p.cargo_process("test").arg("--no-fail-fast"),
                execs().with_status(101)
                       .with_stdout_contains(format!("\
[COMPILING] foo v0.0.1 ([..])
[RUNNING] target[..]foo[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured

[RUNNING] target[..]test_add_one[..]
"))
                       .with_stdout_contains(format!("\
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured

[RUNNING] target[..]test_sub_one[..]

running 1 test
test sub_one_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

{doctest} foo

running 1 test
test sub_one_0 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured

", doctest = DOCTEST)))
});

test!(test_multiple_packages {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.d1]
                path = "d1"
            [dependencies.d2]
                path = "d2"

            [lib]
                name = "foo"
                doctest = false
        "#)
        .file("src/lib.rs", "")
        .file("d1/Cargo.toml", r#"
            [package]
            name = "d1"
            version = "0.0.1"
            authors = []

            [lib]
                name = "d1"
                doctest = false
        "#)
        .file("d1/src/lib.rs", "")
        .file("d2/Cargo.toml", r#"
            [package]
            name = "d2"
            version = "0.0.1"
            authors = []

            [lib]
                name = "d2"
                doctest = false
        "#)
        .file("d2/src/lib.rs", "");
    p.build();

    assert_that(p.cargo("test").arg("-p").arg("d1").arg("-p").arg("d2"),
                execs().with_status(0)
                       .with_stdout_contains(&format!("\
[RUNNING] target[..]debug[..]d1-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured
"))
                       .with_stdout_contains(&format!("\
[RUNNING] target[..]debug[..]d2-[..]

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured
")));
});

test!(bin_does_not_rebuild_tests {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/main.rs", "fn main() {}")
        .file("tests/foo.rs", "");
    p.build();

    assert_that(p.cargo("test").arg("-v"),
                execs().with_status(0));

    ::sleep_ms(1000);
    File::create(&p.root().join("src/main.rs")).unwrap()
         .write_all(b"fn main() { 3; }").unwrap();

    assert_that(p.cargo("test").arg("-v").arg("--no-run"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[COMPILING] foo v0.0.1 ([..])
[RUNNING] `rustc src[..]main.rs [..]`
[RUNNING] `rustc src[..]main.rs [..]`
")));
});

test!(selective_test_wonky_profile {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [profile.release]
            opt-level = 2

            [dependencies]
            a = { path = "a" }
        "#)
        .file("src/lib.rs", "")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#)
        .file("a/src/lib.rs", "");
    p.build();

    assert_that(p.cargo("test").arg("-v").arg("--no-run").arg("--release")
                 .arg("-p").arg("foo").arg("-p").arg("a"),
                execs().with_status(0));
});

test!(selective_test_optional_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = { path = "a", optional = true }
        "#)
        .file("src/lib.rs", "")
        .file("a/Cargo.toml", r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#)
        .file("a/src/lib.rs", "");
    p.build();

    assert_that(p.cargo("test").arg("-v").arg("--no-run")
                 .arg("--features").arg("a").arg("-p").arg("a"),
                execs().with_status(0).with_stdout(&format!("\
[COMPILING] a v0.0.1 ([..])
[RUNNING] `rustc a[..]src[..]lib.rs [..]`
[RUNNING] `rustc a[..]src[..]lib.rs [..]`
")));
});

test!(only_test_docs {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", r#"
            #[test]
            fn foo() {
                let a: u32 = "hello";
            }

            /// ```
            /// println!("ok");
            /// ```
            pub fn bar() {
            }
        "#)
        .file("tests/foo.rs", "this is not rust");
    p.build();

    assert_that(p.cargo("test").arg("--doc"),
                execs().with_status(0));
});
