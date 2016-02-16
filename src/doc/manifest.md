% The Manifest Format - Cargo Documentation

# The `[package]` Section

The first section in a `Cargo.toml` is `[package]`.

```toml
[package]
name = "hello_world" # the name of the package
version = "0.1.0"    # the current version, obeying semver
authors = ["you@example.com"]
```

All three of these fields are mandatory. Cargo bakes in the concept of
[Semantic Versioning](http://semver.org/), so make sure you follow some
basic rules:

* Before you reach 1.0.0, anything goes.
* After 1.0.0, only make breaking changes when you increment the major
  version. In Rust, breaking changes include adding fields to structs or
  variants to enums. Don’t break the build.
* After 1.0.0, don’t add any new public API (no new `pub` anything) in
  tiny versions. Always increment the minor version if you add any new
  `pub` structs, traits, fields, types, functions, methods or anything else.
* Use version numbers with three numeric parts such as 1.0.0 rather than 1.0.

For more on versions, see [this
documentation](crates-io.html#using-cratesio-based-crates).

## The `build` Field (optional)

This field specifies a file in the repository which is a [build
script][1] for building native code. More information can be
found in the build script [guide][1].

[1]: build-script.html

```toml
[package]
# ...
build = "build.rs"
```

## The `exclude` and `include` Fields (optional)

You can explicitly specify to Cargo that a set of [globs][globs] should be ignored or
included for the purposes of packaging and rebuilding a package. The globs
specified in the `exclude` field identify a set of files that are not included
when a package is published as well as ignored for the purposes of detecting
when to rebuild a package, and the globs in `include` specify files that are
explicitly included.

If a VCS is being used for a package, the `exclude` field will be seeded with
the VCS’ ignore settings (`.gitignore` for git for example).

```toml
[package]
# ...
exclude = ["build/**/*.o", "doc/**/*.html"]
```

```toml
[package]
# ...
include = ["src/**/*", "Cargo.toml"]
```

The options are mutually exclusive: setting `include` will override an
`exclude`. Note that `include` must be an exhaustive list of files as otherwise
necessary source files may not be included.

[globs]: http://doc.rust-lang.org/glob/glob/struct.Pattern.html


## The `publish`  Field (optional)

The `publish` field can be used to prevent a package from being
published to a repository by mistake.

```toml
[package]
# ...
publish = false
```

## Package metadata

There are a number of optional metadata fields also accepted under the
`[package]` section:

```toml
[package]
# ...

# A short blurb about the package. This is not rendered in any format when
# uploaded to crates.io (aka this is not markdown)
description = "..."

# These URLs point to more information about the repository
documentation = "..."
homepage = "..."
repository = "..."

# This points to a file in the repository (relative to this Cargo.toml). The
# contents of this file are stored and indexed in the registry.
readme = "..."

# This is a small list of keywords used to categorize and search for this
# package.
keywords = ["...", "..."]

# This is a string description of the license for this package. Currently
# crates.io will validate the license provided against a whitelist of known
# license identifiers from http://spdx.org/licenses/. Multiple licenses can
# be separated with a `/`
license = "..."

# If a project is using a nonstandard license, then this key may be specified in
# lieu of the above key and must point to a file relative to this manifest
# (similar to the readme key)
license-file = "..."
```

The [crates.io](https://crates.io) registry will render the description, display
the license, link to the three URLs and categorize by the keywords. These keys
provide useful information to users of the registry and also influence the
search ranking of a crate. It is highly discouraged to omit everything in a
published crate.


# The `[dependencies]` Section

You list dependencies using keys inside of the `[dependencies]` section. For
example, if you wanted to depend on `hammer`, `color`, and `geometry`:

```toml
[package]
# ...

[dependencies]
hammer = { version = "0.5.0", git = "https://github.com/wycats/hammer.rs" }
color = { git = "https://github.com/bjz/color-rs" }
geometry = { path = "crates/geometry" }
```

You can specify the source of a dependency in a few ways:

* `git = "<git-url>"`: A git repository with a `Cargo.toml` inside it (not
  necessarily at the root). The `rev`, `tag`, and `branch` options are also
  recognized to use something other than the `master` branch.
* `path = "<relative-path>"`: A path relative to the current `Cargo.toml`
  pointing to another directory with a `Cargo.toml` and an associated package.
* If `path` and `git` are omitted, then a dependencies will come from crates.io
  and use the `version` key to indicate the version requirement.

Dependencies from crates.io can also use a shorthand where just the version
requirement is specified:

```toml
[dependencies]
hammer = "0.5.0"
color = "> 0.6.0, < 0.8.0"
```

The syntax of the requirement strings is described in the [crates.io
guide](crates-io.html#using-cratesio-based-crates).

Platform-specific dependencies take the same format, but are listed under the
`target` section. Normally Rust-like `#[cfg]` syntax will be used to define
these sections:

```toml
[target.'cfg(windows)'.dependencies]
winhttp = "0.4.0"

[target.'cfg(unix)'.dependencies]
openssl = "1.0.1"

[target.'cfg(target_pointer_width = "32")'.dependencies]
native = { path = "native/i686" }

[target.'cfg(target_pointer_width = "64")'.dependencies]
native = { path = "native/i686" }
```

Like with Rust, the syntax here supports the `not`, `any`, and `all` operators
to combine various cfg name/value pairs. Note that the `cfg` syntax has only
been available since Cargo 0.9.0 (Rust 1.8.0).

In addition to `#[cfg]` syntax, Cargo also supports listing out the full target
the dependencies would apply to:

```toml
[target.x86_64-pc-windows-gnu.dependencies]
winhttp = "0.4.0"

[target.i686-unknown-linux-gnu.dependencies]
openssl = "1.0.1"
```

If you’re using a custom target specification, quote the full path and file
name:

```toml
[target."x86_64/windows.json".dependencies]
winhttp = "0.4.0"

[target."i686/linux.json".dependencies]
openssl = "1.0.1"
native = { path = "native/i686" }

[target."x86_64/linux.json".dependencies]
openssl = "1.0.1"
native = { path = "native/x86_64" }
```

# The `[profile.*]` Sections

Cargo supports custom configuration of how rustc is invoked through **profiles**
at the top level. Any manifest may declare a profile, but only the **top level**
project’s profiles are actually read. All dependencies’ profiles will be
overridden. This is done so the top-level project has control over how its
dependencies are compiled.

There are five currently supported profile names, all of which have the same
configuration available to them. Listed below is the configuration available,
along with the defaults for each profile.

```toml
# The development profile, used for `cargo build`
[profile.dev]
opt-level = 0  # Controls the --opt-level the compiler builds with
debug = true   # Controls whether the compiler passes `-g`
rpath = false  # Controls whether the compiler passes `-C rpath`
lto = false    # Controls `-C lto` for binaries and staticlibs
debug-assertions = true  # Controls whether debug assertions are enabled
codegen-units = 1 # Controls whether the compiler passes `-C codegen-units`
                  # `codegen-units` is ignored when `lto = true`

# The release profile, used for `cargo build --release`
[profile.release]
opt-level = 3
debug = false
rpath = false
lto = false
debug-assertions = false
codegen-units = 1

# The testing profile, used for `cargo test`
[profile.test]
opt-level = 0
debug = true
rpath = false
lto = false
debug-assertions = true
codegen-units = 1

# The benchmarking profile, used for `cargo bench`
[profile.bench]
opt-level = 3
debug = false
rpath = false
lto = false
debug-assertions = false
codegen-units = 1

# The documentation profile, used for `cargo doc`
[profile.doc]
opt-level = 0
debug = true
rpath = false
lto = false
debug-assertions = true
codegen-units = 1
```

# The `[features]` Section

Cargo supports **features** to allow expression of:

* Conditional compilation options (usable through `cfg` attributes);
* Optional dependencies, which enhance a package, but are not required;
* Clusters of optional dependencies, such as “postgres”, that would include the
  `postgres` package, the `postgres-macros` package, and possibly other packages
  (such as development-time mocking libraries, debugging tools, etc.)

A feature of a package is either an optional dependency, or a set of other
features. The format for specifying features is:

```toml
[package]
name = "awesome"

[features]
# The “default” set of optional packages. Most people will
# want to use these packages, but they are strictly optional.
# Note that `session` is not a package but rather another
# feature listed in this manifest.
default = ["jquery", "uglifier", "session"]

# A feature with no dependencies is used mainly for conditional
# compilation, like `#[cfg(feature = "go-faster")]`.
go-faster = []

# The “secure-password” feature depends on the bcrypt package.
# This aliasing will allow people to talk about the feature in
# a higher-level way and allow this package to add more
# requirements to the feature in the future.
secure-password = ["bcrypt"]

# Features can be used to reexport features of other packages.
# The `session` feature of package `awesome` will ensure that the
# `session` feature of the package `cookie` is also enabled.
session = ["cookie/session"]

[dependencies]
# These packages are mandatory and form the core of this
# package’s distribution
cookie = "1.2.0"
oauth = "1.1.0"
route-recognizer = "=2.1.0"

# A list of all of the optional dependencies, some of which
# are included in the above “features”. They can be opted
# into by apps.
jquery = { version = "1.0.2", optional = true }
uglifier = { version = "1.5.3", optional = true }
bcrypt = { version = "*", optional = true }
civet = { version = "*", optional = true }
```

To use the package `awesome`:

```toml
[dependencies.awesome]
version = "1.3.5"
default-features = false # do not include the default features, and optionally
                         # cherry-pick individual features
features = ["secure-password", "civet"]
```

## Rules

The usage of features is subject to a few rules:

1. Feature names must not conflict with other package names in the manifest.
   This is because they are opted into via `features = [...]`, which only has a
   single namespace
2. With the exception of the `default` feature, all features are opt-in. To opt
   out of the default feature, use `default-features = false` and cherry-pick
   individual features.
3. Feature groups are not allowed to cyclically depend on one another.
4. Dev-dependencies cannot be optional
5. Features groups can only reference optional dependencies
6. When a feature is selected, Cargo will call `rustc` with
   `--cfg feature="${feature_name}"`. If a feature group is included,
   it and all of its individual features will be included. This can be
   tested in code via `#[cfg(feature = "foo")]`

Note that it is explicitly allowed for features to not actually activate any
optional dependencies. This allows packages to internally enable/disable
features without requiring a new dependency.

## Usage In End Products

One major use-case for this feature is specifying optional features in
end-products. For example, the Servo project may want to include optional
features that people can enable or disable when they build it.

In that case, Servo will describe features in its `Cargo.toml` and they can be
enabled using command-line flags:

```
$ cargo build --release --features "shumway pdf"
```

Default features could be excluded using `--no-default-features`.

## Usage In Packages

In most cases, the concept of “optional dependency” in a library is best
expressed as a separate package that the top-level application depends on.

However, high-level packages, like Iron or Piston, may want the ability to
curate a number of packages for easy installation. The current Cargo system
allows them to curate a number of mandatory dependencies into a single package
for easy installation.

In some cases, packages may want to provide additional curation for **optional**
dependencies:

* Grouping a number of low-level optional dependencies together into a single
  high-level “feature”.
* Specifying packages that are recommended (or suggested) to be included by
  users of the package.
* Including a feature (like `secure-password` in the motivating example) that
  will only work if an optional dependency is available, and would be difficult
  to implement as a separate package. For example, it may be overly difficult to
  design an IO package to be completely decoupled from OpenSSL, with opt-in via
  the inclusion of a separate package.

In almost all cases, it is an antipattern to use these features outside of
high-level packages that are designed for curation. If a feature is optional, it
can almost certainly be expressed as a separate package.

# The `[dev-dependencies]` Section

The format of this section is equivalent to `[dependencies]`. Dev-dependencies
are not used when compiling a package for building, but are used for compiling
tests and benchmarks.

These dependencies are *not* propagated to other packages which depend on this
package.

# The Project Layout

If your project is an executable, name the main source file `src/main.rs`.
If it is a library, name the main source file `src/lib.rs`.

Cargo will also treat any files located in `src/bin/*.rs` as
executables.

Your project can optionally contain folders named `examples`, `tests`, and
`benches`, which Cargo will treat as containing example executable files,
integration tests, and benchmarks respectively.

```notrust
▾ src/          # directory containing source files
  lib.rs        # the main entry point for libraries and packages
  main.rs       # the main entry point for projects producing executables
  ▾ bin/        # (optional) directory containing additional executables
    *.rs
▾ examples/     # (optional) examples
  *.rs
▾ tests/        # (optional) integration tests
  *.rs
▾ benches/      # (optional) benchmarks
  *.rs
```

# Examples

Files located under `examples` are example uses of the functionality
provided by the library. When compiled, they are placed in the
`target/examples` directory.

They must compile as executables (with a `main()` function) and load in the
library by using `extern crate <library-name>`. They are compiled when you run
your tests to protect them from bitrotting.

You can run individual examples with the command `cargo run --example <example-name>`.

# Tests

When you run `cargo test`, Cargo will:

* Compile and run your library’s unit tests, which are in files reachable from
  `lib.rs`. Any sections marked with `#[cfg(test)]` will be included.
* Compile and run your library’s documentation tests, which are embedded
  inside of documentation blocks.
* Compile and run your library’s [integration tests](#integration-tests).
* Compile your library’s examples.

## Integration tests

Each file in `tests/*.rs` is an integration test. When you run `cargo test`,
Cargo will compile each of these files as a separate crate. The crate can link
to your library by using `extern crate <library-name>`, like any other code
that depends on it.

Cargo will not automatically compile files inside subdirectories of `tests`,
but an integration test can import modules from these directories as usual.
For example, if you want several integration tests to share some code, you can
put the shared code in `tests/common/mod.rs` and then put `mod common;` in
each of the test files.

# Configuring a target

All of the  `[[bin]]`, `[lib]`, `[[bench]]`, `[[test]]`, and `[[example]]`
sections support similar configuration for specifying how a target should be
built. The example below uses `[lib]`, but it also applies to all other
sections as well. All values listed are the defaults for that option unless
otherwise specified.

```toml
[package]
# ...

[lib]
# The name of a target is the name of the library that will be generated. This
# is defaulted to the name of the package or project.
name = "foo"

# This field points at where the crate is located, relative to the Cargo.toml.
path = "src/lib.rs"

# A flag for enabling unit tests for this target. This is used by `cargo test`.
test = true

# A flag for enabling documentation tests for this target. This is only
# relevant for libraries, it has no effect on other sections. This is used by
# `cargo test`.
doctest = true

# A flag for enabling benchmarks for this target. This is used by `cargo bench`.
bench = true

# A flag for enabling documentation of this target. This is used by `cargo doc`.
doc = true

# If the target is meant to be a compiler plugin, this field must be set to true
# for Cargo to correctly compile it and make it available for all dependencies.
plugin = false

# If set to false, `cargo test` will omit the --test flag to rustc, which stops
# it from generating a test harness. This is useful when the binary being built
# manages the test runner itself.
harness = true
```

# Building Dynamic or Static Libraries

If your project produces a library, you can specify which kind of
library to build by explicitly listing the library in your `Cargo.toml`:

```toml
# ...

[lib]
name = "..."
# this could be “staticlib” as well
crate-type = ["dylib"]
```

The available options are `dylib`, `rlib`, and `staticlib`. You should only use
this option in a project. Cargo will always compile **packages** (dependencies)
based on the requirements of the project that includes them.
