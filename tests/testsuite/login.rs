//! Tests for the `cargo login` command.

use cargo_test_support::install::cargo_home;
use cargo_test_support::registry::RegistryBuilder;
use cargo_test_support::{cargo_process, t};
use std::fs::{self};
use std::path::PathBuf;
use toml_edit::easy as toml;

const TOKEN: &str = "test-token";
const TOKEN2: &str = "test-token2";
const ORIGINAL_TOKEN: &str = "api-token";

fn setup_new_credentials() {
    let config = cargo_home().join("credentials");
    setup_new_credentials_at(config);
}

fn setup_new_credentials_at(config: PathBuf) {
    t!(fs::create_dir_all(config.parent().unwrap()));
    t!(fs::write(
        &config,
        format!(r#"token = "{token}""#, token = ORIGINAL_TOKEN)
    ));
}

fn check_token(expected_token: &str, registry: Option<&str>) -> bool {
    let credentials = cargo_home().join("credentials");
    assert!(credentials.is_file());

    let contents = fs::read_to_string(&credentials).unwrap();
    let toml: toml::Value = contents.parse().unwrap();

    let token = match (registry, toml) {
        // A registry has been provided, so check that the token exists in a
        // table for the registry.
        (Some(registry), toml::Value::Table(table)) => table
            .get("registries")
            .and_then(|registries_table| registries_table.get(registry))
            .and_then(|registry_table| match registry_table.get("token") {
                Some(&toml::Value::String(ref token)) => Some(token.as_str().to_string()),
                _ => None,
            }),
        // There is no registry provided, so check the global token instead.
        (None, toml::Value::Table(table)) => table
            .get("registry")
            .and_then(|registry_table| registry_table.get("token"))
            .and_then(|v| match v {
                toml::Value::String(ref token) => Some(token.as_str().to_string()),
                _ => None,
            }),
        _ => None,
    };

    if let Some(token_val) = token {
        token_val == expected_token
    } else {
        false
    }
}

#[cargo_test]
fn registry_credentials() {
    let _alternative = RegistryBuilder::new().alternative().build();
    let _alternative2 = RegistryBuilder::new()
        .alternative_named("alternative2")
        .build();

    setup_new_credentials();

    let reg = "alternative";

    cargo_process("login --registry").arg(reg).arg(TOKEN).run();

    // Ensure that we have not updated the default token
    assert!(check_token(ORIGINAL_TOKEN, None));

    // Also ensure that we get the new token for the registry
    assert!(check_token(TOKEN, Some(reg)));

    let reg2 = "alternative2";
    cargo_process("login --registry")
        .arg(reg2)
        .arg(TOKEN2)
        .run();

    // Ensure not overwriting 1st alternate registry token with
    // 2nd alternate registry token (see rust-lang/cargo#7701).
    assert!(check_token(ORIGINAL_TOKEN, None));
    assert!(check_token(TOKEN, Some(reg)));
    assert!(check_token(TOKEN2, Some(reg2)));
}

#[cargo_test]
fn empty_login_token() {
    let registry = RegistryBuilder::new()
        .no_configure_registry()
        .no_configure_token()
        .build();
    setup_new_credentials();

    cargo_process("login")
        .replace_crates_io(registry.index_url())
        .with_stdout("please paste the token found on [..]/me below")
        .with_stdin("\t\n")
        .with_stderr(
            "\
[UPDATING] crates.io index
[ERROR] please provide a non-empty token
",
        )
        .with_status(101)
        .run();

    cargo_process("login")
        .replace_crates_io(registry.index_url())
        .arg("")
        .with_stderr(
            "\
[ERROR] please provide a non-empty token
",
        )
        .with_status(101)
        .run();
}

#[cargo_test]
fn bad_asymmetric_token_args() {
    // These cases are kept brief as the implementation is covered by clap, so this is only smoke testing that we have clap configured correctly.
    cargo_process("login --key-subject=foo tok")
        .with_stderr_contains(
            "[ERROR] The argument '--key-subject <SUBJECT>' cannot be used with '[token]'",
        )
        .with_status(1)
        .run();

    cargo_process("login --generate-keypair tok")
        .with_stderr_contains(
            "[ERROR] The argument '--generate-keypair' cannot be used with '[token]'",
        )
        .with_status(1)
        .run();

    cargo_process("login --secret-key tok")
        .with_stderr_contains("[ERROR] The argument '--secret-key' cannot be used with '[token]'")
        .with_status(1)
        .run();

    cargo_process("login --generate-keypair --secret-key")
        .with_stderr_contains(
            "[ERROR] The argument '--generate-keypair' cannot be used with '--secret-key'",
        )
        .with_status(1)
        .run();
}

// todo why do theas hang when run as a test?
// #[cargo_test]
// fn asymmetric_requires_nightly() {
//     cargo_process("login --key-subject=foo")
//         .with_status(101)
//         .with_stderr_contains("asymmetric token options are unstable and require the `-Z registry-auth` option on the nightly channel")
//         .run();
//     cargo_process("login --generate-keypair")
//         .with_status(101)
//         .with_stderr_contains("asymmetric token options are unstable and require the `-Z registry-auth` option on the nightly channel")
//         .run();
//     cargo_process("login --secret-key")
//         .with_status(101)
//         .with_stderr_contains("asymmetric token options are unstable and require the `-Z registry-auth` option on the nightly channel")
//         .run();
// }
