use support::project;

#[test]
fn parses_env() {
    let p = project().file("src/lib.rs", "").build();

    p.cargo("doc -v")
        .env("RUSTDOCFLAGS", "--cfg=foo")
        .with_stderr_contains("[RUNNING] `rustdoc [..] --cfg=foo[..]`")
        .run();
}

#[test]
fn parses_config() {
    let p = project()
        .file("src/lib.rs", "")
        .file(
            ".cargo/config",
            r#"
            [build]
            rustdocflags = ["--cfg", "foo"]
        "#,
        ).build();

    p.cargo("doc -v")
        .with_stderr_contains("[RUNNING] `rustdoc [..] --cfg foo[..]`")
        .run();
}

#[test]
fn bad_flags() {
    let p = project().file("src/lib.rs", "").build();

    p.cargo("doc")
        .env("RUSTDOCFLAGS", "--bogus")
        .with_status(101)
        .run();
}

#[test]
fn rerun() {
    let p = project().file("src/lib.rs", "").build();

    p.cargo("doc").env("RUSTDOCFLAGS", "--cfg=foo").run();
    p.cargo("doc")
        .env("RUSTDOCFLAGS", "--cfg=foo")
        .with_stderr("[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]")
        .run();
    p.cargo("doc")
        .env("RUSTDOCFLAGS", "--cfg=bar")
        .with_stderr(
            "\
[DOCUMENTING] foo v0.0.1 ([..])
[FINISHED] dev [unoptimized + debuginfo] target(s) in [..]
",
        ).run();
}

#[test]
fn rustdocflags_passed_to_rustdoc_through_cargo_test() {
    let p = project()
        .file(
            "src/lib.rs",
            r#"
            //! ```
            //! assert!(cfg!(do_not_choke));
            //! ```
        "#,
        ).build();

    p.cargo("test --doc")
        .env("RUSTDOCFLAGS", "--cfg do_not_choke")
        .run();
}

#[test]
fn rustdocflags_passed_to_rustdoc_through_cargo_test_only_once() {
    let p = project().file("src/lib.rs", "").build();

    p.cargo("test --doc")
        .env("RUSTDOCFLAGS", "--markdown-no-toc")
        .run();
}
