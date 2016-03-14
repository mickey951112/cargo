use std::path::MAIN_SEPARATOR as SEP;

use support::{project, execs, path2url};
use support::{COMPILING, RUNNING, ERROR};
use hamcrest::{assert_that, existing_file};

fn setup() {
}

test!(simple {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { println!("hello"); }
        "#);

    assert_that(p.cargo_process("run"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ({dir})
{running} `target{sep}debug{sep}foo[..]`
hello
",
        compiling = COMPILING,
        running = RUNNING,
        dir = path2url(p.root()),
        sep = SEP)));
    assert_that(&p.bin("foo"), existing_file());
});

test!(simple_quiet {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { println!("hello"); }
        "#);

    assert_that(p.cargo_process("run").arg("-q"),
                execs().with_status(0).with_stdout("\
hello
")
    );
});

test!(simple_quiet_and_verbose {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { println!("hello"); }
        "#);

    assert_that(p.cargo_process("run").arg("-q").arg("-v"),
                execs().with_status(101).with_stderr(&format!("\
{error} cannot set both --verbose and --quiet
",
error = ERROR)));
});

test!(simple_with_args {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() {
                assert_eq!(std::env::args().nth(1).unwrap(), "hello");
                assert_eq!(std::env::args().nth(2).unwrap(), "world");
            }
        "#);

    assert_that(p.cargo_process("run").arg("hello").arg("world"),
                execs().with_status(0));
});

test!(exit_code {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { std::process::exit(2); }
        "#);

    assert_that(p.cargo_process("run"),
                execs().with_status(2)
                       .with_stderr(&format!("\
{error} Process didn't exit successfully: `target[..]foo[..]` (exit code: 2)
",
error = ERROR)));
});

test!(exit_code_verbose {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { std::process::exit(2); }
        "#);

    assert_that(p.cargo_process("run").arg("-v"),
                execs().with_status(2)
                       .with_stderr(&format!("\
{error} Process didn't exit successfully: `target[..]foo[..]` (exit code: 2)
",
error = ERROR)));
});

test!(no_main_file {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "");

    assert_that(p.cargo_process("run"),
                execs().with_status(101)
                       .with_stderr(&format!("{error} a bin target must be available \
                                     for `cargo run`\n", error = ERROR)));
});

test!(too_many_bins {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/bin/a.rs", "")
        .file("src/bin/b.rs", "");

    assert_that(p.cargo_process("run"),
                execs().with_status(101)
                       .with_stderr(&format!("{error} `cargo run` requires that a project only \
                                     have one executable; use the `--bin` option \
                                     to specify which one to run\n", error = ERROR)));
});

test!(specify_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/bin/a.rs", r#"
            extern crate foo;
            fn main() { println!("hello a.rs"); }
        "#)
        .file("src/bin/b.rs", r#"
            extern crate foo;
            fn main() { println!("hello b.rs"); }
        "#);

    assert_that(p.cargo_process("run").arg("--bin").arg("a").arg("-v"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ({dir})
{running} `rustc src[..]lib.rs [..]`
{running} `rustc src[..]a.rs [..]`
{running} `target{sep}debug{sep}a[..]`
hello a.rs
",
        compiling = COMPILING,
        running = RUNNING,
        dir = path2url(p.root()),
        sep = SEP)));

    assert_that(p.cargo("run").arg("--bin").arg("b").arg("-v"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ([..])
{running} `rustc src[..]b.rs [..]`
{running} `target{sep}debug{sep}b[..]`
hello b.rs
",
        running = RUNNING, compiling = COMPILING,
        sep = SEP)));
});

test!(run_example {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("examples/a.rs", r#"
            fn main() { println!("example"); }
        "#)
        .file("src/bin/a.rs", r#"
            fn main() { println!("bin"); }
        "#);

    assert_that(p.cargo_process("run").arg("--example").arg("a"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ({dir})
{running} `target{sep}debug{sep}examples{sep}a[..]`
example
",
        compiling = COMPILING,
        running = RUNNING,
        dir = path2url(p.root()),
        sep = SEP)));
});

test!(run_with_filename {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/bin/a.rs", r#"
            extern crate foo;
            fn main() { println!("hello a.rs"); }
        "#)
        .file("examples/a.rs", r#"
            fn main() { println!("example"); }
        "#);

    assert_that(p.cargo_process("run").arg("--bin").arg("bin.rs"),
                execs().with_status(101).with_stderr(&format!("\
{error} no bin target named `bin.rs`", error = ERROR)));

    assert_that(p.cargo_process("run").arg("--bin").arg("a.rs"),
                execs().with_status(101).with_stderr(&format!("\
{error} no bin target named `a.rs`

Did you mean `a`?", error = ERROR)));

    assert_that(p.cargo_process("run").arg("--example").arg("example.rs"),
                execs().with_status(101).with_stderr(&format!("\
{error} no example target named `example.rs`", error = ERROR)));

    assert_that(p.cargo_process("run").arg("--example").arg("a.rs"),
                execs().with_status(101).with_stderr(&format!("\
{error} no example target named `a.rs`

Did you mean `a`?", error = ERROR)));
});

test!(either_name_or_example {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/bin/a.rs", r#"
            fn main() { println!("hello a.rs"); }
        "#)
        .file("examples/b.rs", r#"
            fn main() { println!("hello b.rs"); }
        "#);

    assert_that(p.cargo_process("run").arg("--bin").arg("a").arg("--example").arg("b"),
                execs().with_status(101)
                       .with_stderr(&format!("{error} `cargo run` can run at most one \
                                     executable, but multiple were \
                                     specified",
                                     error = ERROR)));
});

test!(one_bin_multiple_examples {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/lib.rs", "")
        .file("src/bin/main.rs", r#"
            fn main() { println!("hello main.rs"); }
        "#)
        .file("examples/a.rs", r#"
            fn main() { println!("hello a.rs"); }
        "#)
        .file("examples/b.rs", r#"
            fn main() { println!("hello b.rs"); }
        "#);

    assert_that(p.cargo_process("run"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ({dir})
{running} `target{sep}debug{sep}main[..]`
hello main.rs
",
        compiling = COMPILING,
        running = RUNNING,
        dir = path2url(p.root()),
        sep = SEP)));
});

test!(example_with_release_flag {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            version = "*"
            path = "bar"
        "#)
        .file("examples/a.rs", r#"
            extern crate bar;

            fn main() {
                if cfg!(debug_assertions) {
                    println!("slow1")
                } else {
                    println!("fast1")
                }
                bar::baz();
            }
        "#)
        .file("bar/Cargo.toml", r#"
            [project]
            name = "bar"
            version = "0.0.1"
            authors = []

            [lib]
            name = "bar"
        "#)
        .file("bar/src/bar.rs", r#"
            pub fn baz() {
                if cfg!(debug_assertions) {
                    println!("slow2")
                } else {
                    println!("fast2")
                }
            }
        "#);

    assert_that(p.cargo_process("run").arg("-v").arg("--release").arg("--example").arg("a"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} bar v0.0.1 ({url}/bar)
{running} `rustc bar{sep}src{sep}bar.rs --crate-name bar --crate-type lib \
        -C opt-level=3 \
        -C metadata=[..] \
        -C extra-filename=[..] \
        --out-dir {dir}{sep}target{sep}release{sep}deps \
        --emit=dep-info,link \
        -L dependency={dir}{sep}target{sep}release{sep}deps \
        -L dependency={dir}{sep}target{sep}release{sep}deps`
{compiling} foo v0.0.1 ({url})
{running} `rustc examples{sep}a.rs --crate-name a --crate-type bin \
        -C opt-level=3 \
        --out-dir {dir}{sep}target{sep}release{sep}examples \
        --emit=dep-info,link \
        -L dependency={dir}{sep}target{sep}release \
        -L dependency={dir}{sep}target{sep}release{sep}deps \
         --extern bar={dir}{sep}target{sep}release{sep}deps{sep}libbar-[..].rlib`
{running} `target{sep}release{sep}examples{sep}a[..]`
fast1
fast2
",
        compiling = COMPILING,
        running = RUNNING,
        dir = p.root().display(),
        url = path2url(p.root()),
        sep = SEP)));

    assert_that(p.cargo("run").arg("-v").arg("--example").arg("a"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} bar v0.0.1 ({url}/bar)
{running} `rustc bar{sep}src{sep}bar.rs --crate-name bar --crate-type lib \
        -g \
        -C metadata=[..] \
        -C extra-filename=[..] \
        --out-dir {dir}{sep}target{sep}debug{sep}deps \
        --emit=dep-info,link \
        -L dependency={dir}{sep}target{sep}debug{sep}deps \
        -L dependency={dir}{sep}target{sep}debug{sep}deps`
{compiling} foo v0.0.1 ({url})
{running} `rustc examples{sep}a.rs --crate-name a --crate-type bin \
        -g \
        --out-dir {dir}{sep}target{sep}debug{sep}examples \
        --emit=dep-info,link \
        -L dependency={dir}{sep}target{sep}debug \
        -L dependency={dir}{sep}target{sep}debug{sep}deps \
         --extern bar={dir}{sep}target{sep}debug{sep}deps{sep}libbar-[..].rlib`
{running} `target{sep}debug{sep}examples{sep}a[..]`
slow1
slow2
",
        compiling = COMPILING,
        running = RUNNING,
        dir = p.root().display(),
        url = path2url(p.root()),
        sep = SEP)));
});

test!(run_dylib_dep {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [dependencies.bar]
            path = "bar"
        "#)
        .file("src/main.rs", r#"
            extern crate bar;
            fn main() { bar::bar(); }
        "#)
        .file("bar/Cargo.toml", r#"
            [package]
            name = "bar"
            version = "0.0.1"
            authors = []

            [lib]
            name = "bar"
            crate-type = ["dylib"]
        "#)
        .file("bar/src/lib.rs", "pub fn bar() {}");

    assert_that(p.cargo_process("run").arg("hello").arg("world"),
                execs().with_status(0));
});

test!(release_works {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { if cfg!(debug_assertions) { panic!() } }
        "#);

    assert_that(p.cargo_process("run").arg("--release"),
                execs().with_status(0).with_stdout(&format!("\
{compiling} foo v0.0.1 ({dir})
{running} `target{sep}release{sep}foo[..]`
",
        compiling = COMPILING,
        running = RUNNING,
        dir = path2url(p.root()),
        sep = SEP)));
    assert_that(&p.release_bin("foo"), existing_file());
});

test!(run_bin_different_name {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[bin]]
            name = "bar"
        "#)
        .file("src/bar.rs", r#"
            fn main() { }
        "#);

    assert_that(p.cargo_process("run"), execs().with_status(0));
});

test!(dashes_are_forwarded {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []

            [[bin]]
            name = "bar"
        "#)
        .file("src/main.rs", r#"
            fn main() {
                let s: Vec<String> = std::env::args().collect();
                assert_eq!(s[1], "a");
                assert_eq!(s[2], "--");
                assert_eq!(s[3], "b");
            }
        "#);

    assert_that(p.cargo_process("run").arg("--").arg("a").arg("--").arg("b"),
                execs().with_status(0));
});

test!(run_from_executable_folder {
    let p = project("foo")
        .file("Cargo.toml", r#"
            [project]
            name = "foo"
            version = "0.0.1"
            authors = []
        "#)
        .file("src/main.rs", r#"
            fn main() { println!("hello"); }
        "#);

    let cwd = p.root().join("target").join("debug");
    p.cargo_process("build").exec_with_output().unwrap();

    assert_that(p.cargo("run").cwd(cwd),
                execs().with_status(0).with_stdout(&format!("\
{running} `.{sep}foo[..]`
hello
",
        running = RUNNING,
        sep = SEP
        )));
});
