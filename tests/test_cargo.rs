use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::str;

use cargo_process;
use support::paths;
use support::{execs, project, mkdir_recursive, ProjectBuilder, ERROR};
use hamcrest::{assert_that};

fn setup() {
}

/// Add an empty file with executable flags (and platform-dependent suffix).
/// TODO: move this to `ProjectBuilder` if other cases using this emerge.
fn fake_executable(proj: ProjectBuilder, dir: &Path, name: &str) -> ProjectBuilder {
    let path = proj.root().join(dir).join(&format!("{}{}", name,
                                                   env::consts::EXE_SUFFIX));
    mkdir_recursive(path.parent().unwrap()).unwrap();
    File::create(&path).unwrap();
    make_executable(&path);
    return proj;

    #[cfg(unix)]
    fn make_executable(p: &Path) {
        use std::os::unix::prelude::*;

        let mut perms = fs::metadata(p).unwrap().permissions();;
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        fs::set_permissions(p, perms).unwrap();
    }
    #[cfg(windows)]
    fn make_executable(_: &Path) {}
}

fn path() -> Vec<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or(OsString::new())).collect()
}

test!(list_commands_looks_at_path {
    let proj = project("list-non-overlapping");
    let proj = fake_executable(proj, &Path::new("path-test"), "cargo-1");
    let mut pr = cargo_process();

    let mut path = path();
    path.push(proj.root().join("path-test"));
    let path = env::join_paths(path.iter()).unwrap();
    let output = pr.arg("-v").arg("--list")
                   .env("PATH", &path);
    let output = output.exec_with_output().unwrap();
    let output = str::from_utf8(&output.stdout).unwrap();
    assert!(output.contains("\n    1\n"), "missing 1: {}", output);
});

test!(find_closest_biuld_to_build {
    let mut pr = cargo_process();
    pr.arg("biuld");

    assert_that(pr,
                execs().with_status(101)
                       .with_stderr(&format!("{error} no such subcommand

<tab>Did you mean `build`?

",
error = ERROR)));
});

// if a subcommand is more than 3 edit distance away, we don't make a suggestion
test!(find_closest_dont_correct_nonsense {
    let paths = path().into_iter().filter(|p| {
        fs::read_dir(p).into_iter()
           .flat_map(|i| i)
           .filter_map(|e| e.ok())
           .all(|e| !e.file_name().to_str().unwrap_or("").starts_with("cargo-"))
    });
    let mut pr = cargo_process();
    pr.arg("asdf")
      .cwd(&paths::root())
      .env("PATH", env::join_paths(paths).unwrap());

    assert_that(pr,
                execs().with_status(101)
                       .with_stderr(&format!("{error} no such subcommand
",
error = ERROR)));
});

test!(override_cargo_home {
    let root = paths::root();
    let my_home = root.join("my_home");
    fs::create_dir(&my_home).unwrap();
    File::create(&my_home.join("config")).unwrap().write_all(br#"
        [cargo-new]
        name = "foo"
        email = "bar"
        git = false
    "#).unwrap();

    assert_that(cargo_process()
                    .arg("new").arg("foo")
                    .env("USER", "foo")
                    .env("CARGO_HOME", &my_home),
                execs().with_status(0));

    let toml = paths::root().join("foo/Cargo.toml");
    let mut contents = String::new();
    File::open(&toml).unwrap().read_to_string(&mut contents).unwrap();
    assert!(contents.contains(r#"authors = ["foo <bar>"]"#));
});

test!(cargo_help {
    assert_that(cargo_process(),
                execs().with_status(0));
    assert_that(cargo_process().arg("help"),
                execs().with_status(0));
    assert_that(cargo_process().arg("-h"),
                execs().with_status(0));
    assert_that(cargo_process().arg("help").arg("build"),
                execs().with_status(0));
    assert_that(cargo_process().arg("build").arg("-h"),
                execs().with_status(0));
    assert_that(cargo_process().arg("help").arg("-h"),
                execs().with_status(0));
    assert_that(cargo_process().arg("help").arg("help"),
                execs().with_status(0));
});
