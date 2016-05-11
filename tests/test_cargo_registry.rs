use std::fs::{self, File};
use std::io::prelude::*;

use support::{project, execs};
use support::{DOWNLOADING, PACKAGING, VERIFYING, ADDING, REMOVING};
use support::paths::{self, CargoPathExt};
use support::registry::{self, Package};
use support::git;

use hamcrest::assert_that;

fn setup() {
}

test!(simple {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = ">= 0.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    Package::new("bar", "0.0.1").publish();

    assert_that(p.cargo_process("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `{reg}`
{downloading} bar v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
",
        downloading = DOWNLOADING,
        dir = p.url(),
        reg = registry::registry())));

    // Don't download a second time
    assert_that(p.cargo_process("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `{reg}`
[..] bar v0.0.1 (registry file://[..])
[..] foo v0.0.1 ({dir})
",
        dir = p.url(),
        reg = registry::registry())));
});

test!(deps {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = ">= 0.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    Package::new("baz", "0.0.1").publish();
    Package::new("bar", "0.0.1").dep("baz", "*").publish();

    assert_that(p.cargo_process("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `{reg}`
{downloading} [..] v0.0.1 (registry file://[..])
{downloading} [..] v0.0.1 (registry file://[..])
[COMPILING] baz v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
",
        downloading = DOWNLOADING,
        dir = p.url(),
        reg = registry::registry())));
});

test!(nonexistent {
    Package::new("init", "0.0.1").publish();

    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            nonexistent = ">= 0.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    assert_that(p.cargo_process("build"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `nonexistent` found (required by `foo`)
location searched: registry file://[..]
version required: >= 0.0.0
")));
});

test!(wrong_version {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            foo = ">= 1.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    Package::new("foo", "0.0.1").publish();
    Package::new("foo", "0.0.2").publish();

    assert_that(p.cargo_process("build"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `foo` found (required by `foo`)
location searched: registry file://[..]
version required: >= 1.0.0
versions found: 0.0.2, 0.0.1
")));

    Package::new("foo", "0.0.3").publish();
    Package::new("foo", "0.0.4").publish();

    assert_that(p.cargo_process("build"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `foo` found (required by `foo`)
location searched: registry file://[..]
version required: >= 1.0.0
versions found: 0.0.4, 0.0.3, 0.0.2, ...
")));
});

test!(bad_cksum {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bad-cksum = ">= 0.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    let pkg = Package::new("bad-cksum", "0.0.1");
    pkg.publish();
    File::create(&pkg.archive_dst()).unwrap();

    assert_that(p.cargo_process("build").arg("-v"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] unable to get packages from source

Caused by:
  failed to download package `bad-cksum v0.0.1 (registry file://[..])` from [..]

Caused by:
  failed to verify the checksum of `bad-cksum v0.0.1 (registry file://[..])`
")));
});

test!(update_registry {
    Package::new("init", "0.0.1").publish();

    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            notyet = ">= 0.0.0"
        "#)
        .file("src/main.rs", "fn main() {}");

    assert_that(p.cargo_process("build"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `notyet` found (required by `foo`)
location searched: registry file://[..]
version required: >= 0.0.0
")));

    Package::new("notyet", "0.0.1").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `{reg}`
{downloading} notyet v0.0.1 (registry file://[..])
[COMPILING] notyet v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
",
        downloading = DOWNLOADING,
        dir = p.url(),
        reg = registry::registry())));
});

test!(package_with_path_deps {
    Package::new("init", "0.0.1").publish();

    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
            license = "MIT"
            description = "foo"
            repository = "bar"

            [dependencies.notyet]
            version = "0.0.1"
            path = "notyet"
        "#)
        .file("src/main.rs", "fn main() {}")
        .file("notyet/Cargo.toml", r#"
            [package]
            name = "notyet"
            version = "0.0.1"
            authors = []
        "#)
        .file("notyet/src/lib.rs", "");
    p.build();

    assert_that(p.cargo("package").arg("-v"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] failed to verify package tarball

Caused by:
  no matching package named `notyet` found (required by `foo`)
location searched: registry file://[..]
version required: ^0.0.1
")));

    Package::new("notyet", "0.0.1").publish();

    assert_that(p.cargo("package"),
                execs().with_status(0).with_stdout(format!("\
{packaging} foo v0.0.1 ({dir})
{verifying} foo v0.0.1 ({dir})
[UPDATING] registry `[..]`
{downloading} notyet v0.0.1 (registry file://[..])
[COMPILING] notyet v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir}[..])
",
    packaging = PACKAGING,
    verifying = VERIFYING,
    downloading = DOWNLOADING,
    dir = p.url(),
)));
});

test!(lockfile_locks {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("bar", "0.0.1").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} bar v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

    p.root().move_into_the_past().unwrap();
    Package::new("bar", "0.0.2").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));
});

test!(lockfile_locks_transitively {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("baz", "0.0.1").publish();
    Package::new("bar", "0.0.1").dep("baz", "*").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} [..] v0.0.1 (registry file://[..])
{downloading} [..] v0.0.1 (registry file://[..])
[COMPILING] baz v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

    p.root().move_into_the_past().unwrap();
    Package::new("baz", "0.0.2").publish();
    Package::new("bar", "0.0.2").dep("baz", "*").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));
});

test!(yanks_are_not_used {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("baz", "0.0.1").publish();
    Package::new("baz", "0.0.2").yanked(true).publish();
    Package::new("bar", "0.0.1").dep("baz", "*").publish();
    Package::new("bar", "0.0.2").dep("baz", "*").yanked(true).publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} [..] v0.0.1 (registry file://[..])
{downloading} [..] v0.0.1 (registry file://[..])
[COMPILING] baz v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));
});

test!(relying_on_a_yank_is_bad {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("baz", "0.0.1").publish();
    Package::new("baz", "0.0.2").yanked(true).publish();
    Package::new("bar", "0.0.1").dep("baz", "=0.0.2").publish();

    assert_that(p.cargo("build"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `baz` found (required by `bar`)
location searched: registry file://[..]
version required: = 0.0.2
versions found: 0.0.1
")));
});

test!(yanks_in_lockfiles_are_ok {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("bar", "0.0.1").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0));

    fs::remove_dir_all(&registry::registry_path().join("3")).unwrap();

    Package::new("bar", "0.0.1").yanked(true).publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));

    assert_that(p.cargo("update"),
                execs().with_status(101).with_stderr(&format!("\
[ERROR] no matching package named `bar` found (required by `foo`)
location searched: registry file://[..]
version required: *
")));
});

test!(update_with_lockfile_if_packages_missing {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("bar", "0.0.1").publish();
    assert_that(p.cargo("build"),
                execs().with_status(0));
    p.root().move_into_the_past().unwrap();

    paths::home().join(".cargo/registry").rm_rf().unwrap();
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} bar v0.0.1 (registry file://[..])
", downloading = DOWNLOADING)));
});

test!(update_lockfile {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    println!("0.0.1");
    Package::new("bar", "0.0.1").publish();
    assert_that(p.cargo("build"),
                execs().with_status(0));

    Package::new("bar", "0.0.2").publish();
    Package::new("bar", "0.0.3").publish();
    paths::home().join(".cargo/registry").rm_rf().unwrap();
    println!("0.0.2 update");
    assert_that(p.cargo("update")
                 .arg("-p").arg("bar").arg("--precise").arg("0.0.2"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
[UPDATING] bar v0.0.1 (registry file://[..]) -> v0.0.2
")));

    println!("0.0.2 build");
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
{downloading} [..] v0.0.2 (registry file://[..])
[COMPILING] bar v0.0.2 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

    println!("0.0.3 update");
    assert_that(p.cargo("update")
                 .arg("-p").arg("bar"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
[UPDATING] bar v0.0.2 (registry file://[..]) -> v0.0.3
")));

    println!("0.0.3 build");
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
{downloading} [..] v0.0.3 (registry file://[..])
[COMPILING] bar v0.0.3 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

   println!("new dependencies update");
   Package::new("bar", "0.0.4").dep("spam", "0.2.5").publish();
   Package::new("spam", "0.2.5").publish();
   assert_that(p.cargo("update")
                .arg("-p").arg("bar"),
               execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
[UPDATING] bar v0.0.3 (registry file://[..]) -> v0.0.4
{adding} spam v0.2.5 (registry file://[..])
", adding = ADDING)));

   println!("new dependencies update");
   Package::new("bar", "0.0.5").publish();
   assert_that(p.cargo("update")
                .arg("-p").arg("bar"),
               execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
[UPDATING] bar v0.0.4 (registry file://[..]) -> v0.0.5
{removing} spam v0.2.5 (registry file://[..])
", removing = REMOVING)));
});

test!(dev_dependency_not_used {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("baz", "0.0.1").publish();
    Package::new("bar", "0.0.1").dev_dep("baz", "*").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} [..] v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));
});

test!(login_with_no_cargo_dir {
    let home = paths::home().join("new-home");
    fs::create_dir(&home).unwrap();
    assert_that(::cargo_process().arg("login").arg("foo").arg("-v"),
                execs().with_status(0));
});

test!(bad_license_file {
    Package::new("foo", "1.0.0").publish();
    let p = project("all")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
            license-file = "foo"
            description = "bar"
            repository = "baz"
        "#)
        .file("src/main.rs", r#"
            fn main() {}
        "#);
    assert_that(p.cargo_process("publish").arg("-v"),
                execs().with_status(101)
                       .with_stderr(&format!("\
[ERROR] the license file `foo` does not exist")));
});

test!(updating_a_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.a]
            path = "a"
        "#)
        .file("src/main.rs", "fn main() {}")
        .file("a/Cargo.toml", r#"
            [project]
            name = "a"
            version = "0.0.1"
            authors = []

            [dependencies]
            bar = "*"
        "#)
        .file("a/src/lib.rs", "");
    p.build();

    Package::new("bar", "0.0.1").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} bar v0.0.1 (registry file://[..])
[COMPILING] bar v0.0.1 (registry file://[..])
[COMPILING] a v0.0.1 ({dir}/a)
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

    File::create(&p.root().join("a/Cargo.toml")).unwrap().write_all(br#"
        [project]
        name = "a"
        version = "0.0.1"
        authors = []

        [dependencies]
        bar = "0.1.0"
    "#).unwrap();
    Package::new("bar", "0.1.0").publish();

    println!("second");
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} bar v0.1.0 (registry file://[..])
[COMPILING] bar v0.1.0 (registry file://[..])
[COMPILING] a v0.0.1 ({dir}/a)
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));
});

test!(git_and_registry_dep {
    let b = git::repo(&paths::root().join("b"))
        .file("Cargo.toml", r#"
            [project]
            name = "b"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = "0.0.1"
        "#)
        .file("src/lib.rs", "");
    b.build();
    let p = project("foo")
        .file("Cargo.toml", &format!(r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies]
            a = "0.0.1"

            [dependencies.b]
            git = '{}'
        "#, b.url()))
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("a", "0.0.1").publish();

    p.root().move_into_the_past().unwrap();
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] [..]
[UPDATING] [..]
{downloading} a v0.0.1 (registry file://[..])
[COMPILING] a v0.0.1 (registry [..])
[COMPILING] b v0.0.1 ([..])
[COMPILING] foo v0.0.1 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));
    p.root().move_into_the_past().unwrap();

    println!("second");
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(""));
});

test!(update_publish_then_update {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            a = "0.1.0"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("a", "0.1.0").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0));

    Package::new("a", "0.1.1").publish();

    let lock = p.root().join("Cargo.lock");
    let mut s = String::new();
    File::open(&lock).unwrap().read_to_string(&mut s).unwrap();
    File::create(&lock).unwrap()
         .write_all(s.replace("0.1.0", "0.1.1").as_bytes()).unwrap();
    println!("second");

    fs::remove_dir_all(&p.root().join("target")).unwrap();
    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] [..]
{downloading} a v0.1.1 (registry file://[..])
[COMPILING] a v0.1.1 (registry [..])
[COMPILING] foo v0.5.0 ({dir})
", downloading = DOWNLOADING,
   dir = p.url())));

});

test!(fetch_downloads {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            a = "0.1.0"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("a", "0.1.0").publish();

    assert_that(p.cargo("fetch"),
                execs().with_status(0)
                       .with_stdout(format!("\
[UPDATING] registry `[..]`
{downloading} a v0.1.0 (registry [..])
", downloading = DOWNLOADING)));
});

test!(update_transitive_dependency {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            a = "0.1.0"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("a", "0.1.0").dep("b", "*").publish();
    Package::new("b", "0.1.0").publish();

    assert_that(p.cargo("fetch"),
                execs().with_status(0));

    Package::new("b", "0.1.1").publish();

    assert_that(p.cargo("update").arg("-pb"),
                execs().with_status(0)
                       .with_stdout(format!("\
[UPDATING] registry `[..]`
[UPDATING] b v0.1.0 (registry [..]) -> v0.1.1
")));

    assert_that(p.cargo("build"),
                execs().with_status(0)
                       .with_stdout(format!("\
{downloading} b v0.1.1 (registry file://[..])
[COMPILING] b v0.1.1 (registry [..])
[COMPILING] a v0.1.0 (registry [..])
[COMPILING] foo v0.5.0 ([..])
", downloading = DOWNLOADING)));
});

test!(update_backtracking_ok {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            webdriver = "0.1"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("webdriver", "0.1.0").dep("hyper", "0.6").publish();
    Package::new("hyper", "0.6.5").dep("openssl", "0.1")
                                  .dep("cookie", "0.1")
                                  .publish();
    Package::new("cookie", "0.1.0").dep("openssl", "0.1").publish();
    Package::new("openssl", "0.1.0").publish();

    assert_that(p.cargo("generate-lockfile"),
                execs().with_status(0));

    Package::new("openssl", "0.1.1").publish();
    Package::new("hyper", "0.6.6").dep("openssl", "0.1.1")
                                  .dep("cookie", "0.1.0")
                                  .publish();

    assert_that(p.cargo("update").arg("-p").arg("hyper"),
                execs().with_status(0)
                       .with_stdout(&format!("\
[UPDATING] registry `[..]`
")));
});

test!(update_multiple_packages {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            a = "*"
            b = "*"
            c = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("a", "0.1.0").publish();
    Package::new("b", "0.1.0").publish();
    Package::new("c", "0.1.0").publish();

    assert_that(p.cargo("fetch"),
                execs().with_status(0));

    Package::new("a", "0.1.1").publish();
    Package::new("b", "0.1.1").publish();
    Package::new("c", "0.1.1").publish();

    assert_that(p.cargo("update").arg("-pa").arg("-pb"),
                execs().with_status(0)
                       .with_stdout(format!("\
[UPDATING] registry `[..]`
[UPDATING] a v0.1.0 (registry [..]) -> v0.1.1
[UPDATING] b v0.1.0 (registry [..]) -> v0.1.1
")));

    assert_that(p.cargo("update").arg("-pb").arg("-pc"),
                execs().with_status(0)
                       .with_stdout(format!("\
[UPDATING] registry `[..]`
[UPDATING] c v0.1.0 (registry [..]) -> v0.1.1
")));

    assert_that(p.cargo("build"),
                execs().with_status(0)
                       .with_stdout_contains(format!("\
{downloading} a v0.1.1 (registry file://[..])", downloading = DOWNLOADING))
                       .with_stdout_contains(format!("\
{downloading} b v0.1.1 (registry file://[..])", downloading = DOWNLOADING))
                       .with_stdout_contains(format!("\
{downloading} c v0.1.1 (registry file://[..])", downloading = DOWNLOADING))
                       .with_stdout_contains(format!("\
[COMPILING] a v0.1.1 (registry [..])"))
                       .with_stdout_contains(format!("\
[COMPILING] b v0.1.1 (registry [..])"))
                       .with_stdout_contains(format!("\
[COMPILING] c v0.1.1 (registry [..])"))
                       .with_stdout_contains(format!("\
[COMPILING] foo v0.5.0 ([..])")));
});

test!(bundled_crate_in_registry {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.5.0"
            authors = []

            [dependencies]
            bar = "0.1"
            baz = "0.1"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("bar", "0.1.0").publish();
    Package::new("baz", "0.1.0")
        .dep("bar", "0.1.0")
        .file("Cargo.toml", r#"
            [package]
            name = "baz"
            version = "0.1.0"
            authors = []

            [dependencies]
            bar = { path = "bar", version = "0.1.0" }
        "#)
        .file("src/lib.rs", "")
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.1.0"
            authors = []
        "#)
        .file("bar/src/lib.rs", "")
        .publish();

    assert_that(p.cargo("run"), execs().with_status(0));
});

test!(update_same_prefix_oh_my_how_was_this_a_bug {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "ugh"
            version = "0.5.0"
            authors = []

            [dependencies]
            foo = "0.1"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("foobar", "0.2.0").publish();
    Package::new("foo", "0.1.0")
        .dep("foobar", "0.2.0")
        .publish();

    assert_that(p.cargo("generate-lockfile"), execs().with_status(0));
    assert_that(p.cargo("update").arg("-pfoobar").arg("--precise=0.2.0"),
                execs().with_status(0));
});

test!(use_semver {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "bar"
            version = "0.5.0"
            authors = []

            [dependencies]
            foo = "1.2.3-alpha.0"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("foo", "1.2.3-alpha.0").publish();

    assert_that(p.cargo("build"), execs().with_status(0));
});

test!(only_download_relevant {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "bar"
            version = "0.5.0"
            authors = []

            [target.foo.dependencies]
            foo = "*"
            [dev-dependencies]
            bar = "*"
            [dependencies]
            baz = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("foo", "0.1.0").publish();
    Package::new("bar", "0.1.0").publish();
    Package::new("baz", "0.1.0").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0).with_stdout(&format!("\
[UPDATING] registry `[..]`
{downloading} baz v0.1.0 ([..])
[COMPILING] baz v0.1.0 ([..])
[COMPILING] bar v0.5.0 ([..])
", downloading = DOWNLOADING)));
});

test!(resolve_and_backtracking {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "bar"
            version = "0.5.0"
            authors = []

            [dependencies]
            foo = "*"
        "#)
        .file("src/main.rs", "fn main() {}");
    p.build();

    Package::new("foo", "0.1.1")
            .feature_dep("bar", "0.1", &["a", "b"])
            .publish();
    Package::new("foo", "0.1.0").publish();

    assert_that(p.cargo("build"),
                execs().with_status(0));
});
