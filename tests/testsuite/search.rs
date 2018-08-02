use std::fs::{self, File};
use std::io::prelude::*;
use std::path::Path;

use support::{cargo_process, execs};
use support::git::repo;
use support::paths;
use support::registry::{api_path, registry as registry_url, registry_path};
use support::hamcrest::assert_that;
use url::Url;

fn api() -> Url {
    Url::from_file_path(&*api_path()).ok().unwrap()
}

fn write_crates(dest: &Path) {
    let content = r#"{
        "crates": [{
            "created_at": "2014-11-16T20:17:35Z",
            "description": "Design by contract style assertions for Rust",
            "documentation": null,
            "downloads": 2,
            "homepage": null,
            "id": "hoare",
            "keywords": [],
            "license": null,
            "links": {
                "owners": "/api/v1/crates/hoare/owners",
                "reverse_dependencies": "/api/v1/crates/hoare/reverse_dependencies",
                "version_downloads": "/api/v1/crates/hoare/downloads",
                "versions": "/api/v1/crates/hoare/versions"
            },
            "max_version": "0.1.1",
            "name": "hoare",
            "repository": "https://github.com/nick29581/libhoare",
            "updated_at": "2014-11-20T21:49:21Z",
            "versions": null
        }],
        "meta": {
            "total": 1
        }
    }"#;

    // Older versions of curl don't peel off query parameters when looking for
    // filenames, so just make both files.
    //
    // On windows, though, `?` is an invalid character, but we always build curl
    // from source there anyway!
    File::create(&dest)
        .unwrap()
        .write_all(content.as_bytes())
        .unwrap();
    if !cfg!(windows) {
        File::create(&dest.with_file_name("crates?q=postgres&per_page=10"))
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
    }
}

fn setup() {
    let cargo_home = paths::root().join(".cargo");
    fs::create_dir_all(cargo_home).unwrap();
    fs::create_dir_all(&api_path().join("api/v1")).unwrap();

    // Init a new registry
    let _ = repo(&registry_path())
        .file("config.json", &format!(r#"{{"dl":"{0}","api":"{0}"}}"#, api()))
        .build();

    let base = api_path().join("api/v1/crates");
    write_crates(&base);
}

fn set_cargo_config() {
    let config = paths::root().join(".cargo/config");

    File::create(&config)
        .unwrap()
        .write_all(
            format!(
                r#"
[source.crates-io]
registry = 'https://wut'
replace-with = 'dummy-registry'

[source.dummy-registry]
registry = '{reg}'
"#,
                reg = registry_url(),
            ).as_bytes(),
        )
        .unwrap();
}

#[test]
fn not_update() {
    setup();
    set_cargo_config();

    use cargo::core::{Shell, Source, SourceId};
    use cargo::sources::RegistrySource;
    use cargo::util::Config;

    let sid = SourceId::for_registry(&registry_url()).unwrap();
    let cfg = Config::new(Shell::new(), paths::root(), paths::home().join(".cargo"));
    let mut regsrc = RegistrySource::remote(&sid, &cfg);
    regsrc.update().unwrap();

    assert_that(
        cargo_process("search postgres"),
        execs()
            .with_status(0)
            .with_stdout_contains(
                "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
            )
            .with_stderr(""), // without "Updating registry ..."
    );
}

#[test]
fn replace_default() {
    setup();
    set_cargo_config();

    assert_that(
        cargo_process("search postgres"),
        execs()
            .with_status(0)
            .with_stdout_contains(
                "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
            )
            .with_stderr_contains("[..]Updating registry[..]"),
    );
}

#[test]
fn simple() {
    setup();

    assert_that(
        cargo_process("search postgres --index").arg(registry_url().to_string()),
        execs().with_status(0).with_stdout_contains(
            "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
        ),
    );
}

// TODO: Deprecated
// remove once it has been decided '--host' can be safely removed
#[test]
fn simple_with_host() {
    setup();

    assert_that(
        cargo_process("search postgres --host").arg(registry_url().to_string()),
        execs()
            .with_status(0)
            .with_stderr(&format!(
                "\
[WARNING] The flag '--host' is no longer valid.

Previous versions of Cargo accepted this flag, but it is being
deprecated. The flag is being renamed to 'index', as the flag
wants the location of the index. Please use '--index' instead.

This will soon become a hard error, so it's either recommended
to update to a fixed version or contact the upstream maintainer
about this warning.
[UPDATING] registry `{reg}`
",
                reg = registry_url()
            ))
            .with_stdout_contains(
                "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
            ),
    );
}

// TODO: Deprecated
// remove once it has been decided '--host' can be safely removed
#[test]
fn simple_with_index_and_host() {
    setup();

    assert_that(
        cargo_process("search postgres --index").arg(registry_url().to_string()).arg("--host").arg(registry_url().to_string()),
        execs()
            .with_status(0)
            .with_stderr(&format!(
                "\
[WARNING] The flag '--host' is no longer valid.

Previous versions of Cargo accepted this flag, but it is being
deprecated. The flag is being renamed to 'index', as the flag
wants the location of the index. Please use '--index' instead.

This will soon become a hard error, so it's either recommended
to update to a fixed version or contact the upstream maintainer
about this warning.
[UPDATING] registry `{reg}`
",
                reg = registry_url()
            ))
            .with_stdout_contains(
                "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
            ),
    );
}

#[test]
fn multiple_query_params() {
    setup();

    assert_that(
        cargo_process("search postgres sql --index").arg(registry_url().to_string()),
        execs().with_status(0).with_stdout_contains(
            "hoare = \"0.1.1\"    # Design by contract style assertions for Rust",
        ),
    );
}

#[test]
fn help() {
    assert_that(cargo_process("search -h"), execs().with_status(0));
    assert_that(cargo_process("help search"), execs().with_status(0));
    // Ensure that help output goes to stdout, not stderr.
    assert_that(
        cargo_process("search --help"),
        execs().with_stderr(""),
    );
    assert_that(
        cargo_process("search --help"),
        execs().with_stdout_contains("[..] --frozen [..]"),
    );
}
