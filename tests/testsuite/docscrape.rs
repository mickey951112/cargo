//! Tests for the `cargo doc` command.

use cargo_test_support::is_nightly;
use cargo_test_support::project;

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn basic() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("examples/ex.rs", "fn main() { foo::foo(); }")
        .file("src/lib.rs", "pub fn foo() {}\npub fn bar() { foo(); }")
        .build();

    p.cargo("doc -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
            "\
[CHECKING] foo v0.0.1 ([CWD])
[SCRAPING] foo v0.0.1 ([CWD])
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        )
        .run();

    let doc_html = p.read_file("target/doc/foo/fn.foo.html");
    assert!(doc_html.contains("Examples found in repository"));
    assert!(doc_html.contains("More examples"));

    // Ensure that the reverse-dependency has its sources generated
    assert!(p.build_dir().join("doc/src/ex/ex.rs.html").exists());
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn avoid_build_script_cycle() {
    if !is_nightly() {
        // -Z rustdoc-scrape-examples is unstable
        return;
    }

    let p = project()
        // package with build dependency
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
                links = "foo"

                [workspace]
                members = ["bar"]

                [build-dependencies]
                bar = {path = "bar"}
            "#,
        )
        .file("src/lib.rs", "")
        .file("build.rs", "fn main(){}")
        // dependency
        .file(
            "bar/Cargo.toml",
            r#"
                [package]
                name = "bar"
                version = "0.0.1"
                authors = []
                links = "bar"
            "#,
        )
        .file("bar/src/lib.rs", "")
        .file("bar/build.rs", "fn main(){}")
        .build();

    p.cargo("doc --all -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn complex_reverse_dependencies() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []

                [dev-dependencies]
                a = {path = "a", features = ["feature"]}
                b = {path = "b"}

                [workspace]
                members = ["b"]
            "#,
        )
        .file("src/lib.rs", "")
        .file("examples/ex.rs", "fn main() {}")
        .file(
            "a/Cargo.toml",
            r#"
                [package]
                name = "a"
                version = "0.0.1"
                authors = []

                [lib]
                proc-macro = true

                [dependencies]
                b = {path = "../b"}

                [features]
                feature = []
            "#,
        )
        .file("a/src/lib.rs", "#[cfg(feature)] pub fn f();")
        .file(
            "b/Cargo.toml",
            r#"
                [package]
                name = "b"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("b/src/lib.rs", "")
        .build();

    p.cargo("doc --all --examples -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn crate_with_dash() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "da-sh"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("src/lib.rs", "pub fn foo() {}")
        .file("examples/a.rs", "fn main() { da_sh::foo(); }")
        .build();

    p.cargo("doc -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();

    let doc_html = p.read_file("target/doc/da_sh/fn.foo.html");
    assert!(doc_html.contains("Examples found in repository"));
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn configure_target() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []

                [lib]
                doc-scrape-examples = false

                [[bin]]
                name = "a_bin"
                doc-scrape-examples = true
            "#,
        )
        .file(
            "src/lib.rs",
            "pub fn foo() {} fn lib_must_not_appear() { foo(); }",
        )
        .file("examples/a.rs", "fn example_must_appear() { foo::foo(); }")
        .file(
            "src/bin/a_bin.rs",
            "fn bin_must_appear() { foo::foo(); } fn main(){}",
        )
        .build();

    p.cargo("doc --lib --bins -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();

    let doc_html = p.read_file("target/doc/foo/fn.foo.html");
    assert!(!doc_html.contains("lib_must_not_appear"));
    assert!(doc_html.contains("example_must_appear"));
    assert!(doc_html.contains("bin_must_appear"));
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn configure_profile() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []

                [profile.dev]
                panic = "abort"
            "#,
        )
        .file("examples/ex.rs", "fn main() { foo::foo(); }")
        .file("src/lib.rs", "pub fn foo() {}\npub fn bar() { foo(); }")
        .build();

    p.cargo("doc -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();

    let doc_html = p.read_file("target/doc/foo/fn.foo.html");
    assert!(doc_html.contains("Examples found in repository"));
    assert!(doc_html.contains("More examples"));
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn issue_10545() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [workspace]
                resolver = "2"
                members = ["a", "b"]
            "#,
        )
        .file(
            "a/Cargo.toml",
            r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
            edition = "2021"

            [features]
            default = ["foo"]
            foo = []
        "#,
        )
        .file("a/src/lib.rs", "")
        .file(
            "b/Cargo.toml",
            r#"
                [package]
                name = "b"
                version = "0.0.1"
                authors = []
                edition = "2021"

                [lib]
                proc-macro = true
            "#,
        )
        .file("b/src/lib.rs", "")
        .build();

    p.cargo("doc --all -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn cache() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("examples/ex.rs", "fn main() { foo::foo(); }")
        .file("src/lib.rs", "pub fn foo() {}\npub fn bar() { foo(); }")
        .build();

    p.cargo("doc -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
            "\
[CHECKING] foo v0.0.1 ([CWD])
[SCRAPING] foo v0.0.1 ([CWD])
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        )
        .run();

    p.cargo("doc -Zunstable-options -Zrustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
            "\
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        )
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn no_fail_bad_lib() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("src/lib.rs", "pub fn foo() { CRASH_THE_BUILD() }")
        .build();

    p.cargo("doc -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
        "\
[SCRAPING] foo v0.0.1 ([CWD])
warning: failed to scan lib in package `foo` for example code usage
    Try running with `--verbose` to see the error message.
    If this example should not be scanned, consider adding `doc-scrape-examples = false` to the `[[example]]` definition in Cargo.toml
warning: `foo` (lib) generated 1 warning
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
    )
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn no_fail_bad_example() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                authors = []
            "#,
        )
        .file("examples/ex1.rs", "DOES NOT COMPILE")
        .file("examples/ex2.rs", "fn main() { foo::foo(); }")
        .file("src/lib.rs", "pub fn foo(){}")
        .build();

    p.cargo("doc -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
            "\
[CHECKING] foo v0.0.1 ([CWD])
[SCRAPING] foo v0.0.1 ([CWD])
warning: failed to scan example \"ex1\" in package `foo` for example code usage
    Try running with `--verbose` to see the error message.
    If this example should not be scanned, consider adding `doc-scrape-examples = false` to the `[[example]]` definition in Cargo.toml
warning: `foo` (example \"ex1\") generated 1 warning
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
        )
        .run();

    p.cargo("clean").run();

    p.cargo("doc -v -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr_unordered(
            "\
[CHECKING] foo v0.0.1 ([CWD])
[RUNNING] `rustc --crate-name foo[..]
[SCRAPING] foo v0.0.1 ([CWD])
[RUNNING] `rustdoc[..] --crate-name ex1[..]
[RUNNING] `rustdoc[..] --crate-name ex2[..]
[RUNNING] `rustdoc[..] --crate-name foo[..]
error: expected one of `!` or `::`, found `NOT`
 --> examples/ex1.rs:1:6
  |
1 | DOES NOT COMPILE
  |      ^^^ expected one of `!` or `::`

[DOCUMENTING] foo v0.0.1 ([CWD])
[RUNNING] `rustdoc[..] --crate-name foo[..]
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
        )
        .run();

    let doc_html = p.read_file("target/doc/foo/fn.foo.html");
    assert!(doc_html.contains("Examples found in repository"));
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn no_scrape_with_dev_deps() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            doc-scrape-examples = false

            [dev-dependencies]
            a = {path = "a"}
        "#,
        )
        .file("src/lib.rs", "")
        .file("examples/ex.rs", "fn main() { a::f(); }")
        .file(
            "a/Cargo.toml",
            r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("a/src/lib.rs", "pub fn f() {}")
        .build();

    // If --examples is not provided, then the example is never scanned.
    p.cargo("doc -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr(
            "\
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
        )
        .run();

    // If --examples is provided, then the bad example is scanned and ignored.
    p.cargo("doc --examples -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr_unordered(
            "\
[CHECKING] a v0.0.1 ([CWD]/a)
[CHECKING] foo v0.0.1 ([CWD])
[DOCUMENTING] a v0.0.1 ([CWD]/a)
[SCRAPING] foo v0.0.1 ([CWD])
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
        )
        .run();
}

#[cargo_test(nightly, reason = "rustdoc scrape examples flags are unstable")]
fn use_dev_deps_if_explicitly_enabled() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [package]
            name = "foo"
            version = "0.0.1"
            authors = []

            [lib]
            doc-scrape-examples = false

            [[example]]
            name = "ex"
            doc-scrape-examples = true

            [dev-dependencies]
            a = {path = "a"}
        "#,
        )
        .file("src/lib.rs", "")
        .file("examples/ex.rs", "fn main() { a::f(); }")
        .file(
            "a/Cargo.toml",
            r#"
            [package]
            name = "a"
            version = "0.0.1"
            authors = []
        "#,
        )
        .file("a/src/lib.rs", "pub fn f() {}")
        .build();

    // If --examples is not provided, then the example is never scanned.
    p.cargo("doc -Zunstable-options -Z rustdoc-scrape-examples")
        .masquerade_as_nightly_cargo(&["rustdoc-scrape-examples"])
        .with_stderr_unordered(
            "\
[CHECKING] foo v0.0.1 ([CWD])
[CHECKING] a v0.0.1 ([CWD]/a)
[SCRAPING] foo v0.0.1 ([CWD])
[DOCUMENTING] foo v0.0.1 ([CWD])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]",
        )
        .run();
}
