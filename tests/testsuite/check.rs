use support::install::exe;
use support::is_nightly;
use support::paths::CargoPathExt;
use support::registry::Package;
use support::{execs, project};
use glob::glob;
use support::hamcrest::{assert_that, existing_file, is_not};

#[test]
fn check_success() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::baz();
            }
        "#,
        )
        .build();
    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(foo.cargo("check"), execs().with_status(0));
}

#[test]
fn check_fail() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::baz(42);
            }
        "#,
        )
        .build();
    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(foo.cargo("check"), execs().with_status(101));
}

#[test]
fn custom_derive() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
#[macro_use]
extern crate bar;

trait B {
    fn b(&self);
}

#[derive(B)]
struct A;

fn main() {
    let a = A;
    a.b();
}
"#,
        )
        .build();
    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
            [lib]
            proc-macro = true
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
extern crate proc_macro;

use proc_macro::TokenStream;

#[proc_macro_derive(B)]
pub fn derive(_input: TokenStream) -> TokenStream {
    format!("impl B for A {{ fn b(&self) {{}} }}").parse().unwrap()
}
"#,
        )
        .build();

    assert_that(foo.cargo("check"), execs().with_status(0));
}

#[test]
fn check_build() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::baz();
            }
        "#,
        )
        .build();

    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(foo.cargo("check"), execs().with_status(0));
    assert_that(foo.cargo("build"), execs().with_status(0));
}

#[test]
fn build_check() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::baz();
            }
        "#,
        )
        .build();

    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(foo.cargo("build"), execs().with_status(0));
    assert_that(foo.cargo("check"), execs().with_status(0));
}

// Checks that where a project has both a lib and a bin, the lib is only checked
// not built.
#[test]
fn issue_3418() {
    let foo = project()
        .file("src/lib.rs", "")
        .file("src/main.rs", "fn main() {}")
        .build();

    assert_that(
        foo.cargo("check").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --emit=dep-info,metadata [..]"),
    );
}

// Some weirdness that seems to be caused by a crate being built as well as
// checked, but in this case with a proc macro too.
#[test]
fn issue_3419() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            rustc-serialize = "*"
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            extern crate rustc_serialize;

            use rustc_serialize::Decodable;

            pub fn take<T: Decodable>() {}
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate rustc_serialize;

            extern crate foo;

            #[derive(RustcDecodable)]
            pub struct Foo;

            fn main() {
                foo::take::<Foo>();
            }
        "#,
        )
        .build();

    Package::new("rustc-serialize", "1.0.0")
        .file(
            "src/lib.rs",
            r#"pub trait Decodable: Sized {
                    fn decode<D: Decoder>(d: &mut D) -> Result<Self, D::Error>;
                 }
                 pub trait Decoder {
                    type Error;
                    fn read_struct<T, F>(&mut self, s_name: &str, len: usize, f: F)
                                         -> Result<T, Self::Error>
                    where F: FnOnce(&mut Self) -> Result<T, Self::Error>;
                 } "#,
        )
        .publish();

    assert_that(p.cargo("check"), execs().with_status(0));
}

// Check on a dylib should have a different metadata hash than build.
#[test]
fn dylib_check_preserves_build_cache() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.1.0"
            authors = []

            [lib]
            crate-type = ["dylib"]

            [dependencies]
        "#,
        )
        .file("src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("build"),
        execs().with_status(0).with_stderr(
            "\
[..]Compiling foo v0.1.0 ([..])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        ),
    );

    assert_that(p.cargo("check"), execs().with_status(0));

    assert_that(
        p.cargo("build"),
        execs()
            .with_status(0)
            .with_stderr("[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]"),
    );
}

// test `cargo rustc --profile check`
#[test]
fn rustc_check() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::baz();
            }
        "#,
        )
        .build();
    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(
        foo.cargo("rustc")
            .arg("--profile")
            .arg("check")
            .arg("--")
            .arg("--emit=metadata"),
        execs().with_status(0),
    );
}

#[test]
fn rustc_check_err() {
    let foo = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "../bar"
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            extern crate bar;
            fn main() {
                ::bar::qux();
            }
        "#,
        )
        .build();
    let _bar = project().at("bar")
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#,
        )
        .file(
            "src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(
        foo.cargo("rustc")
            .arg("--profile")
            .arg("check")
            .arg("--")
            .arg("--emit=metadata"),
        execs().with_status(101),
    );
}

#[test]
fn check_all() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [workspace]
            [dependencies]
            b = { path = "b" }
        "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file("examples/a.rs", "fn main() {}")
        .file("tests/a.rs", "")
        .file("src/lib.rs", "")
        .file(
            "b/Cargo.toml",
            r#"
            [package]
            name = "b"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("b/src/main.rs", "fn main() {}")
        .file("b/src/lib.rs", "")
        .build();

    assert_that(
        p.cargo("check").arg("--all").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --crate-name foo src[/]lib.rs [..]")
            .with_stderr_contains("[..] --crate-name foo src[/]main.rs [..]")
            .with_stderr_contains("[..] --crate-name b b[/]src[/]lib.rs [..]")
            .with_stderr_contains("[..] --crate-name b b[/]src[/]main.rs [..]"),
    );
}

#[test]
fn check_virtual_all_implied() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [workspace]
            members = ["bar", "baz"]
        "#,
        )
        .file(
            "bar/Cargo.toml",
            r#"
            [project]
            name = "bar"
            version = "0.1.0"
        "#,
        )
        .file(
            "bar/src/lib.rs",
            r#"
            pub fn bar() {}
        "#,
        )
        .file(
            "baz/Cargo.toml",
            r#"
            [project]
            name = "baz"
            version = "0.1.0"
        "#,
        )
        .file(
            "baz/src/lib.rs",
            r#"
            pub fn baz() {}
        "#,
        )
        .build();

    assert_that(
        p.cargo("check").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --crate-name bar bar[/]src[/]lib.rs [..]")
            .with_stderr_contains("[..] --crate-name baz baz[/]src[/]lib.rs [..]"),
    );
}

#[test]
fn targets_selected_default() {
    let foo = project()
        .file("src/main.rs", "fn main() {}")
        .file("src/lib.rs", "pub fn smth() {}")
        .file("examples/example1.rs", "fn main() {}")
        .file("tests/test2.rs", "#[test] fn t() {}")
        .file("benches/bench3.rs", "")
        .build();

    assert_that(
        foo.cargo("check").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --crate-name foo src[/]lib.rs [..]")
            .with_stderr_contains("[..] --crate-name foo src[/]main.rs [..]")
            .with_stderr_does_not_contain("[..] --crate-name example1 examples[/]example1.rs [..]")
            .with_stderr_does_not_contain("[..] --crate-name test2 tests[/]test2.rs [..]")
            .with_stderr_does_not_contain("[..] --crate-name bench3 benches[/]bench3.rs [..]"),
    );
}

#[test]
fn targets_selected_all() {
    let foo = project()
        .file("src/main.rs", "fn main() {}")
        .file("src/lib.rs", "pub fn smth() {}")
        .file("examples/example1.rs", "fn main() {}")
        .file("tests/test2.rs", "#[test] fn t() {}")
        .file("benches/bench3.rs", "")
        .build();

    assert_that(
        foo.cargo("check").arg("--all-targets").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --crate-name foo src[/]lib.rs [..]")
            .with_stderr_contains("[..] --crate-name foo src[/]main.rs [..]")
            .with_stderr_contains("[..] --crate-name example1 examples[/]example1.rs [..]")
            .with_stderr_contains("[..] --crate-name test2 tests[/]test2.rs [..]")
            .with_stderr_contains("[..] --crate-name bench3 benches[/]bench3.rs [..]"),
    );
}

#[test]
fn check_unit_test_profile() {
    let foo = project()
        .file(
            "src/lib.rs",
            r#"
            #[cfg(test)]
            mod tests {
                #[test]
                fn it_works() {
                    badtext
                }
            }
        "#,
        )
        .build();

    assert_that(foo.cargo("check"), execs().with_status(0));
    assert_that(
        foo.cargo("check").arg("--profile").arg("test"),
        execs()
            .with_status(101)
            .with_stderr_contains("[..]badtext[..]"),
    );
}

// Verify what is checked with various command-line filters.
#[test]
fn check_filters() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
            fn unused_normal_lib() {}
            #[cfg(test)]
            mod tests {
                fn unused_unit_lib() {}
            }
        "#,
        )
        .file(
            "src/main.rs",
            r#"
            fn main() {}
            fn unused_normal_bin() {}
            #[cfg(test)]
            mod tests {
                fn unused_unit_bin() {}
            }
        "#,
        )
        .file(
            "tests/t1.rs",
            r#"
            fn unused_normal_t1() {}
            #[cfg(test)]
            mod tests {
                fn unused_unit_t1() {}
            }
        "#,
        )
        .file(
            "examples/ex1.rs",
            r#"
            fn main() {}
            fn unused_normal_ex1() {}
            #[cfg(test)]
            mod tests {
                fn unused_unit_ex1() {}
            }
        "#,
        )
        .file(
            "benches/b1.rs",
            r#"
            fn unused_normal_b1() {}
            #[cfg(test)]
            mod tests {
                fn unused_unit_b1() {}
            }
        "#,
        )
        .build();

    assert_that(
        p.cargo("check"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..]unused_normal_lib[..]")
            .with_stderr_contains("[..]unused_normal_bin[..]")
            .with_stderr_does_not_contain("[..]unused_normal_t1[..]")
            .with_stderr_does_not_contain("[..]unused_normal_ex1[..]")
            .with_stderr_does_not_contain("[..]unused_normal_b1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_[..]"),
    );
    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--tests").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..] --crate-name foo src[/]lib.rs [..] --test [..]")
            .with_stderr_contains("[..] --crate-name foo src[/]lib.rs --crate-type lib [..]")
            .with_stderr_contains("[..] --crate-name foo src[/]main.rs [..] --test [..]")
            .with_stderr_contains("[..]unused_unit_lib[..]")
            .with_stderr_contains("[..]unused_unit_bin[..]")
            .with_stderr_contains("[..]unused_normal_lib[..]")
            .with_stderr_contains("[..]unused_normal_bin[..]")
            .with_stderr_contains("[..]unused_unit_t1[..]")
            .with_stderr_does_not_contain("[..]unused_normal_ex1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_ex1[..]")
            .with_stderr_does_not_contain("[..]unused_normal_b1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_b1[..]")
            .with_stderr_does_not_contain("[..]--crate-type bin[..]"),
    );
    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--test").arg("t1").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..]unused_normal_lib[..]")
            .with_stderr_contains("[..]unused_unit_t1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_lib[..]")
            .with_stderr_does_not_contain("[..]unused_normal_bin[..]")
            .with_stderr_does_not_contain("[..]unused_unit_bin[..]")
            .with_stderr_does_not_contain("[..]unused_normal_ex1[..]")
            .with_stderr_does_not_contain("[..]unused_normal_b1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_ex1[..]")
            .with_stderr_does_not_contain("[..]unused_unit_b1[..]"),
    );
    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--all-targets").arg("-v"),
        execs()
            .with_status(0)
            .with_stderr_contains("[..]unused_normal_lib[..]")
            .with_stderr_contains("[..]unused_normal_bin[..]")
            .with_stderr_contains("[..]unused_normal_t1[..]")
            .with_stderr_contains("[..]unused_normal_ex1[..]")
            .with_stderr_contains("[..]unused_normal_b1[..]")
            .with_stderr_contains("[..]unused_unit_b1[..]")
            .with_stderr_contains("[..]unused_unit_t1[..]")
            .with_stderr_contains("[..]unused_unit_lib[..]")
            .with_stderr_contains("[..]unused_unit_bin[..]")
            .with_stderr_does_not_contain("[..]unused_unit_ex1[..]"),
    );
}

#[test]
fn check_artifacts() {
    // Verify which artifacts are created when running check (#4059).
    let p = project()
        .file("src/lib.rs", "")
        .file("src/main.rs", "fn main() {}")
        .file("tests/t1.rs", "")
        .file("examples/ex1.rs", "fn main() {}")
        .file("benches/b1.rs", "")
        .build();
    assert_that(p.cargo("check"), execs().with_status(0));
    assert_that(&p.root().join("target/debug/libfoo.rmeta"), existing_file());
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug").join(exe("foo")),
        is_not(existing_file()),
    );

    p.root().join("target").rm_rf();
    assert_that(p.cargo("check").arg("--lib"), execs().with_status(0));
    assert_that(&p.root().join("target/debug/libfoo.rmeta"), existing_file());
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug").join(exe("foo")),
        is_not(existing_file()),
    );

    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--bin").arg("foo"),
        execs().with_status(0),
    );
    if is_nightly() {
        // The nightly check can be removed once 1.27 is stable.
        // Bins now generate `rmeta` files.
        // See: https://github.com/rust-lang/rust/pull/49289
        assert_that(&p.root().join("target/debug/libfoo.rmeta"), existing_file());
    }
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug").join(exe("foo")),
        is_not(existing_file()),
    );

    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--test").arg("t1"),
        execs().with_status(0),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rmeta"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug").join(exe("foo")),
        is_not(existing_file()),
    );
    assert_eq!(
        glob(&p.root().join("target/debug/t1-*").to_str().unwrap())
            .unwrap()
            .count(),
        0
    );

    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--example").arg("ex1"),
        execs().with_status(0),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rmeta"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug/examples").join(exe("ex1")),
        is_not(existing_file()),
    );

    p.root().join("target").rm_rf();
    assert_that(
        p.cargo("check").arg("--bench").arg("b1"),
        execs().with_status(0),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rmeta"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug/libfoo.rlib"),
        is_not(existing_file()),
    );
    assert_that(
        &p.root().join("target/debug").join(exe("foo")),
        is_not(existing_file()),
    );
    assert_eq!(
        glob(&p.root().join("target/debug/b1-*").to_str().unwrap())
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn proc_macro() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "demo"
                version = "0.0.1"

                [lib]
                proc-macro = true
            "#,
        )
        .file(
            "src/lib.rs",
            r#"
                extern crate proc_macro;

                use proc_macro::TokenStream;

                #[proc_macro_derive(Foo)]
                pub fn demo(_input: TokenStream) -> TokenStream {
                    "".parse().unwrap()
                }
            "#,
        )
        .file(
            "src/main.rs",
            r#"
                #[macro_use]
                extern crate demo;

                #[derive(Foo)]
                struct A;

                fn main() {}
            "#,
        )
        .build();
    assert_that(
        p.cargo("check").arg("-v").env("RUST_LOG", "cargo=trace"),
        execs().with_status(0),
    );
}
