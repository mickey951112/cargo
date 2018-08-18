use support::{execs, project};
use support::hamcrest::assert_that;

#[test]
fn net_retry_loads_from_config() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            git = "https://127.0.0.1:11/foo/bar"
        "#,
        )
        .file("src/main.rs", "")
        .file(
            ".cargo/config",
            r#"
        [net]
        retry=1
        [http]
        timeout=1
         "#,
        )
        .build();

    assert_that(
        p.cargo("build -v"),
        execs().with_status(101).with_stderr_contains(
            "[WARNING] spurious network error \
             (1 tries remaining): [..]",
        ),
    );
}

#[test]
fn net_retry_git_outputs_warning() {
    let p = project()
        .file(
            "Cargo.toml",
            r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            git = "https://127.0.0.1:11/foo/bar"
        "#,
        )
        .file(
            ".cargo/config",
            r#"
        [http]
        timeout=1
         "#,
        )
        .file("src/main.rs", "")
        .build();

    assert_that(
        p.cargo("build -v -j 1"),
        execs()
            .with_status(101)
            .with_stderr_contains(
                "[WARNING] spurious network error \
                 (2 tries remaining): [..]",
            )
            .with_stderr_contains("[WARNING] spurious network error (1 tries remaining): [..]"),
    );
}
