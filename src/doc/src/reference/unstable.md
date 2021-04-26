## Unstable Features

Experimental Cargo features are only available on the [nightly channel]. You
are encouraged to experiment with these features to see if they meet your
needs, and if there are any issues or problems. Check the linked tracking
issues listed below for more information on the feature, and click the GitHub
subscribe button if you want future updates.

After some period of time, if the feature does not have any major concerns, it
can be [stabilized], which will make it available on stable once the current
nightly release reaches the stable channel (anywhere from 6 to 12 weeks).

There are three different ways that unstable features can be enabled based on
how the feature works:

* New syntax in `Cargo.toml` requires a `cargo-features` key at the top of
  `Cargo.toml`, before any tables. For example:

  ```toml
  # This specifies which new Cargo.toml features are enabled.
  cargo-features = ["test-dummy-unstable"]

  [package]
  name = "my-package"
  version = "0.1.0"
  im-a-teapot = true  # This is a new option enabled by test-dummy-unstable.
  ```

* New command-line flags, options, and subcommands require the `-Z
  unstable-options` CLI option to also be included. For example, the new
  `--out-dir` option is only available on nightly:

  ```cargo +nightly build --out-dir=out -Z unstable-options```

* `-Z` command-line flags are used to enable new functionality that may not
  have an interface, or the interface has not yet been designed, or for more
  complex features that affect multiple parts of Cargo. For example, the
  [timings](#timings) feature can be enabled with:

  ```cargo +nightly build -Z timings```

  Run `cargo -Z help` to see a list of flags available.

  Anything which can be configured with a `-Z` flag can also be set in the
  cargo [config file] (`.cargo/config.toml`) in the `unstable` table. For
  example:

  ```toml
  [unstable]
  mtime-on-use = true
  multitarget = true
  timings = ["html"]
  ```

Each new feature described below should explain how to use it.

[config file]: config.md
[nightly channel]: ../../book/appendix-07-nightly-rust.html
[stabilized]: https://doc.crates.io/contrib/process/unstable.html#stabilization

### allow-features

This permanently-unstable flag makes it so that only a listed set of
unstable features can be used. Specifically, if you pass
`-Zallow-features=foo,bar`, you'll continue to be able to pass `-Zfoo`
and `-Zbar` to `cargo`, but you will be unable to pass `-Zbaz`. You can
pass an empty string (`-Zallow-features=`) to disallow all unstable
features.

`-Zallow-features` also restricts which unstable features can be passed
to the `cargo-features` entry in `Cargo.toml`. If, for example, you want
to allow

```toml
cargo-features = ["test-dummy-unstable"]
```

where `test-dummy-unstable` is unstable, that features would also be
disallowed by `-Zallow-features=`, and allowed with
`-Zallow-features=test-dummy-unstable`.

The list of features passed to cargo's `-Zallow-features` is also passed
to any Rust tools that cargo ends up calling (like `rustc` or
`rustdoc`). Thus, if you run `cargo -Zallow-features=`, no unstable
Cargo _or_ Rust features can be used.

### extra-link-arg
* Original Pull Request: [#7811](https://github.com/rust-lang/cargo/pull/7811)

The `-Z extra-link-arg` flag makes the following two instructions available
in build scripts:

* [`cargo:rustc-link-arg-bins=FLAG`](#rustc-link-arg-bins) – Passes custom
  flags to a linker for binaries.
* [`cargo:rustc-link-arg=FLAG`](#rustc-link-arg) – Passes custom flags to a
  linker for benchmarks, binaries, `cdylib` crates, examples, and tests.

<a id="rustc-link-arg-bins"></a>
#### `cargo:rustc-link-arg-bins=FLAG`

The `rustc-link-arg-bins` instruction tells Cargo to pass the [`-C
link-arg=FLAG` option][link-arg] to the compiler, but only when building a
binary target. Its usage is highly platform specific. It is useful
to set a linker script or other linker options.

[link-arg]: ../../rustc/codegen-options/index.md#link-arg

<a id="rustc-link-arg"></a>
#### `cargo:rustc-link-arg=FLAG`

The `rustc-link-arg` instruction tells Cargo to pass the [`-C link-arg=FLAG`
option][link-arg] to the compiler, but only when building supported targets
(benchmarks, binaries, `cdylib` crates, examples, and tests). Its usage is
highly platform specific. It is useful to set the shared library version or
linker script.

[link-arg]: ../../rustc/codegen-options/index.md#link-arg

### no-index-update
* Original Issue: [#3479](https://github.com/rust-lang/cargo/issues/3479)
* Tracking Issue: [#7404](https://github.com/rust-lang/cargo/issues/7404)

The `-Z no-index-update` flag ensures that Cargo does not attempt to update
the registry index. This is intended for tools such as Crater that issue many
Cargo commands, and you want to avoid the network latency for updating the
index each time.

### mtime-on-use
* Original Issue: [#6477](https://github.com/rust-lang/cargo/pull/6477)
* Cache usage meta tracking issue: [#7150](https://github.com/rust-lang/cargo/issues/7150)

The `-Z mtime-on-use` flag is an experiment to have Cargo update the mtime of
used files to make it easier for tools like cargo-sweep to detect which files
are stale. For many workflows this needs to be set on *all* invocations of cargo.
To make this more practical setting the `unstable.mtime_on_use` flag in `.cargo/config.toml`
or the corresponding ENV variable will apply the `-Z mtime-on-use` to all
invocations of nightly cargo. (the config flag is ignored by stable)

### avoid-dev-deps
* Original Issue: [#4988](https://github.com/rust-lang/cargo/issues/4988)
* Stabilization Issue: [#5133](https://github.com/rust-lang/cargo/issues/5133)

When running commands such as `cargo install` or `cargo build`, Cargo
currently requires dev-dependencies to be downloaded, even if they are not
used. The `-Z avoid-dev-deps` flag allows Cargo to avoid downloading
dev-dependencies if they are not needed. The `Cargo.lock` file will not be
generated if dev-dependencies are skipped.

### minimal-versions
* Original Issue: [#4100](https://github.com/rust-lang/cargo/issues/4100)
* Tracking Issue: [#5657](https://github.com/rust-lang/cargo/issues/5657)

> Note: It is not recommended to use this feature. Because it enforces minimal
> versions for all transitive dependencies, its usefulness is limited since
> not all external dependencies declare proper lower version bounds. It is
> intended that it will be changed in the future to only enforce minimal
> versions for direct dependencies.

When a `Cargo.lock` file is generated, the `-Z minimal-versions` flag will
resolve the dependencies to the minimum semver version that will satisfy the
requirements (instead of the greatest version).

The intended use-case of this flag is to check, during continuous integration,
that the versions specified in Cargo.toml are a correct reflection of the
minimum versions that you are actually using. That is, if Cargo.toml says
`foo = "1.0.0"` that you don't accidentally depend on features added only in
`foo 1.5.0`.

### out-dir
* Original Issue: [#4875](https://github.com/rust-lang/cargo/issues/4875)
* Tracking Issue: [#6790](https://github.com/rust-lang/cargo/issues/6790)

This feature allows you to specify the directory where artifacts will be
copied to after they are built. Typically artifacts are only written to the
`target/release` or `target/debug` directories. However, determining the
exact filename can be tricky since you need to parse JSON output. The
`--out-dir` flag makes it easier to predictably access the artifacts. Note
that the artifacts are copied, so the originals are still in the `target`
directory. Example:

```sh
cargo +nightly build --out-dir=out -Z unstable-options
```

This can also be specified in `.cargo/config.toml` files.

```toml
[build]
out-dir = "out"
```

### doctest-xcompile
* Tracking Issue: [#7040](https://github.com/rust-lang/cargo/issues/7040)
* Tracking Rustc Issue: [#64245](https://github.com/rust-lang/rust/issues/64245)

This flag changes `cargo test`'s behavior when handling doctests when
a target is passed. Currently, if a target is passed that is different
from the host cargo will simply skip testing doctests. If this flag is
present, cargo will continue as normal, passing the tests to doctest,
while also passing it a `--target` option, as well as enabling
`-Zunstable-features --enable-per-target-ignores` and passing along
information from `.cargo/config.toml`. See the rustc issue for more information.

```sh
cargo test --target foo -Zdoctest-xcompile
```

### multitarget
* Tracking Issue: [#8176](https://github.com/rust-lang/cargo/issues/8176)

This flag allows passing multiple `--target` flags to the `cargo` subcommand
selected. When multiple `--target` flags are passed the selected build targets
will be built for each of the selected architectures.

For example to compile a library for both 32 and 64-bit:

```
cargo build --target x86_64-unknown-linux-gnu --target i686-unknown-linux-gnu
```

or running tests for both targets:

```
cargo test --target x86_64-unknown-linux-gnu --target i686-unknown-linux-gnu
```

### Custom named profiles

* Tracking Issue: [rust-lang/cargo#6988](https://github.com/rust-lang/cargo/issues/6988)
* RFC: [#2678](https://github.com/rust-lang/rfcs/pull/2678)

With this feature you can define custom profiles having new names. With the
custom profile enabled, build artifacts can be emitted by default to
directories other than `release` or `debug`, based on the custom profile's
name.

For example:

```toml
cargo-features = ["named-profiles"]

[profile.release-lto]
inherits = "release"
lto = true
````

An `inherits` key is used in order to receive attributes from other profiles,
so that a new custom profile can be based on the standard `dev` or `release`
profile presets. Cargo emits errors in case `inherits` loops are detected. When
considering inheritance hierarchy, all profiles directly or indirectly inherit
from either from `release` or from `dev`.

Valid profile names are: must not be empty, use only alphanumeric characters or
`-` or `_`.

Passing `--profile` with the profile's name to various Cargo commands, directs
operations to use the profile's attributes. Overrides that are specified in the
profiles from which the custom profile inherits are inherited too.

For example, using `cargo build` with `--profile` and the manifest from above:

```sh
cargo +nightly build --profile release-lto -Z unstable-options
```

When a custom profile is used, build artifacts go to a different target by
default. In the example above, you can expect to see the outputs under
`target/release-lto`.


#### New `dir-name` attribute

Some of the paths generated under `target/` have resulted in a de-facto "build
protocol", where `cargo` is invoked as a part of a larger project build. So, to
preserve the existing behavior, there is also a new attribute `dir-name`, which
when left unspecified, defaults to the name of the profile. For example:

```toml
[profile.release-lto]
inherits = "release"
dir-name = "lto"  # Emits to target/lto instead of target/release-lto
lto = true
```


### Namespaced features
* Original issue: [#1286](https://github.com/rust-lang/cargo/issues/1286)
* Tracking Issue: [#5565](https://github.com/rust-lang/cargo/issues/5565)

The `namespaced-features` option makes two changes to how features can be
specified:

* Features may now be defined with the same name as a dependency.
* Optional dependencies can be explicitly enabled in the `[features]` table
  with the `dep:` prefix, which enables the dependency without enabling a
  feature of the same name.

By default, an optional dependency `foo` will define a feature `foo =
["dep:foo"]` *unless* `dep:foo` is mentioned in any other feature, or the
`foo` feature is already defined. This helps prevent unnecessary boilerplate
of listing every optional dependency, but still allows you to override the
implicit feature.

This allows two use cases that were previously not possible:

* You can "hide" an optional dependency, so that external users cannot
  explicitly enable that optional dependency.
* There is no longer a need to create "funky" feature names to work around the
  restriction that features cannot shadow dependency names.

To enable namespaced-features, use the `-Z namespaced-features` command-line
flag.

An example of hiding an optional dependency:

```toml
[dependencies]
regex = { version = "1.4.1", optional = true }
lazy_static = { version = "1.4.0", optional = true }

[features]
regex = ["dep:regex", "dep:lazy_static"]
```

In this example, the "regex" feature enables both `regex` and `lazy_static`.
The `lazy_static` feature does not exist, and a user cannot explicitly enable
it. This helps hide internal details of how your package is implemented.

An example of avoiding "funky" names:

```toml
[dependencies]
bigdecimal = "0.1"
chrono = "0.4"
num-bigint = "0.2"
serde = {version = "1.0", optional = true }

[features]
serde = ["dep:serde", "bigdecimal/serde", "chrono/serde", "num-bigint/serde"]
```

In this case, `serde` is a natural name to use for a feature, because it is
relevant to your exported API. However, previously you would need to use a
name like `serde1` to work around the naming limitation if you wanted to also
enable other features.

### Build-plan
* Tracking Issue: [#5579](https://github.com/rust-lang/cargo/issues/5579)

The `--build-plan` argument for the `build` command will output JSON with
information about which commands would be run without actually executing
anything. This can be useful when integrating with another build tool.
Example:

```sh
cargo +nightly build --build-plan -Z unstable-options
```

### Metabuild
* Tracking Issue: [rust-lang/rust#49803](https://github.com/rust-lang/rust/issues/49803)
* RFC: [#2196](https://github.com/rust-lang/rfcs/blob/master/text/2196-metabuild.md)

Metabuild is a feature to have declarative build scripts. Instead of writing
a `build.rs` script, you specify a list of build dependencies in the
`metabuild` key in `Cargo.toml`. A build script is automatically generated
that runs each build dependency in order. Metabuild packages can then read
metadata from `Cargo.toml` to specify their behavior.

Include `cargo-features` at the top of `Cargo.toml`, a `metabuild` key in the
`package`, list the dependencies in `build-dependencies`, and add any metadata
that the metabuild packages require under `package.metadata`. Example:

```toml
cargo-features = ["metabuild"]

[package]
name = "mypackage"
version = "0.0.1"
metabuild = ["foo", "bar"]

[build-dependencies]
foo = "1.0"
bar = "1.0"

[package.metadata.foo]
extra-info = "qwerty"
```

Metabuild packages should have a public function called `metabuild` that
performs the same actions as a regular `build.rs` script would perform.

### public-dependency
* Tracking Issue: [#44663](https://github.com/rust-lang/rust/issues/44663)

The 'public-dependency' feature allows marking dependencies as 'public'
or 'private'. When this feature is enabled, additional information is passed to rustc to allow
the 'exported_private_dependencies' lint to function properly.

This requires the appropriate key to be set in `cargo-features`:

```toml
cargo-features = ["public-dependency"]

[dependencies]
my_dep = { version = "1.2.3", public = true }
private_dep = "2.0.0" # Will be 'private' by default
```

### build-std
* Tracking Repository: https://github.com/rust-lang/wg-cargo-std-aware

The `build-std` feature enables Cargo to compile the standard library itself as
part of a crate graph compilation. This feature has also historically been known
as "std-aware Cargo". This feature is still in very early stages of development,
and is also a possible massive feature addition to Cargo. This is a very large
feature to document, even in the minimal form that it exists in today, so if
you're curious to stay up to date you'll want to follow the [tracking
repository](https://github.com/rust-lang/wg-cargo-std-aware) and its set of
issues.

The functionality implemented today is behind a flag called `-Z build-std`. This
flag indicates that Cargo should compile the standard library from source code
using the same profile as the main build itself. Note that for this to work you
need to have the source code for the standard library available, and at this
time the only supported method of doing so is to add the `rust-src` rust rustup
component:

```console
$ rustup component add rust-src --toolchain nightly
```

It is also required today that the `-Z build-std` flag is combined with the
`--target` flag. Note that you're not forced to do a cross compilation, you're
just forced to pass `--target` in one form or another.

Usage looks like:

```console
$ cargo new foo
$ cd foo
$ cargo +nightly run -Z build-std --target x86_64-unknown-linux-gnu
   Compiling core v0.0.0 (...)
   ...
   Compiling foo v0.1.0 (...)
    Finished dev [unoptimized + debuginfo] target(s) in 21.00s
     Running `target/x86_64-unknown-linux-gnu/debug/foo`
Hello, world!
```

Here we recompiled the standard library in debug mode with debug assertions
(like `src/main.rs` is compiled) and everything was linked together at the end.

Using `-Z build-std` will implicitly compile the stable crates `core`, `std`,
`alloc`, and `proc_macro`. If you're using `cargo test` it will also compile the
`test` crate. If you're working with an environment which does not support some
of these crates, then you can pass an argument to `-Zbuild-std` as well:

```console
$ cargo +nightly build -Z build-std=core,alloc
```

The value here is a comma-separated list of standard library crates to build.

#### Requirements

As a summary, a list of requirements today to use `-Z build-std` are:

* You must install libstd's source code through `rustup component add rust-src`
* You must pass `--target`
* You must use both a nightly Cargo and a nightly rustc
* The `-Z build-std` flag must be passed to all `cargo` invocations.

#### Reporting bugs and helping out

The `-Z build-std` feature is in the very early stages of development! This
feature for Cargo has an extremely long history and is very large in scope, and
this is just the beginning. If you'd like to report bugs please either report
them to:

* Cargo - https://github.com/rust-lang/cargo/issues/new - for implementation bugs
* The tracking repository -
  https://github.com/rust-lang/wg-cargo-std-aware/issues/new - for larger design
  questions.

Also if you'd like to see a feature that's not yet implemented and/or if
something doesn't quite work the way you'd like it to, feel free to check out
the [issue tracker](https://github.com/rust-lang/wg-cargo-std-aware/issues) of
the tracking repository, and if it's not there please file a new issue!

### build-std-features
* Tracking Repository: https://github.com/rust-lang/wg-cargo-std-aware

This flag is a sibling to the `-Zbuild-std` feature flag. This will configure
the features enabled for the standard library itself when building the standard
library. The default enabled features, at this time, are `backtrace` and
`panic_unwind`. This flag expects a comma-separated list and, if provided, will
override the default list of features enabled.

### timings
* Tracking Issue: [#7405](https://github.com/rust-lang/cargo/issues/7405)

The `timings` feature gives some information about how long each compilation
takes, and tracks concurrency information over time.

```sh
cargo +nightly build -Z timings
```

The `-Ztimings` flag can optionally take a comma-separated list of the
following values:

- `html` — Saves a file called `cargo-timing.html` to the current directory
  with a report of the compilation. Files are also saved with a timestamp in
  the filename if you want to look at older runs.
- `info` — Displays a message to stdout after each compilation finishes with
  how long it took.
- `json` — Emits some JSON information about timing information.

The default if none are specified is `html,info`.

#### Reading the graphs

There are two graphs in the output. The "unit" graph shows the duration of
each unit over time. A "unit" is a single compiler invocation. There are lines
that show which additional units are "unlocked" when a unit finishes. That is,
it shows the new units that are now allowed to run because their dependencies
are all finished. Hover the mouse over a unit to highlight the lines. This can
help visualize the critical path of dependencies. This may change between runs
because the units may finish in different orders.

The "codegen" times are highlighted in a lavender color. In some cases, build
pipelining allows units to start when their dependencies are performing code
generation. This information is not always displayed (for example, binary
units do not show when code generation starts).

The "custom build" units are `build.rs` scripts, which when run are
highlighted in orange.

The second graph shows Cargo's concurrency over time. The three lines are:
- "Waiting" (red) — This is the number of units waiting for a CPU slot to
  open.
- "Inactive" (blue) — This is the number of units that are waiting for their
  dependencies to finish.
- "Active" (green) — This is the number of units currently running.

Note: This does not show the concurrency in the compiler itself. `rustc`
coordinates with Cargo via the "job server" to stay within the concurrency
limit. This currently mostly applies to the code generation phase.

Tips for addressing compile times:
- Look for slow dependencies.
    - Check if they have features that you may wish to consider disabling.
    - Consider trying to remove the dependency completely.
- Look for a crate being built multiple times with different versions. Try to
  remove the older versions from the dependency graph.
- Split large crates into smaller pieces.
- If there are a large number of crates bottlenecked on a single crate, focus
  your attention on improving that one crate to improve parallelism.

### binary-dep-depinfo
* Tracking rustc issue: [#63012](https://github.com/rust-lang/rust/issues/63012)

The `-Z binary-dep-depinfo` flag causes Cargo to forward the same flag to
`rustc` which will then cause `rustc` to include the paths of all binary
dependencies in the "dep info" file (with the `.d` extension). Cargo then uses
that information for change-detection (if any binary dependency changes, then
the crate will be rebuilt). The primary use case is for building the compiler
itself, which has implicit dependencies on the standard library that would
otherwise be untracked for change-detection.

### panic-abort-tests

The `-Z panic-abort-tests` flag will enable nightly support to compile test
harness crates with `-Cpanic=abort`. Without this flag Cargo will compile tests,
and everything they depend on, with `-Cpanic=unwind` because it's the only way
`test`-the-crate knows how to operate. As of [rust-lang/rust#64158], however,
the `test` crate supports `-C panic=abort` with a test-per-process, and can help
avoid compiling crate graphs multiple times.

It's currently unclear how this feature will be stabilized in Cargo, but we'd
like to stabilize it somehow!

[rust-lang/rust#64158]: https://github.com/rust-lang/rust/pull/64158

### config-cli
* Tracking Issue: [#7722](https://github.com/rust-lang/cargo/issues/7722)

The `--config` CLI option allows arbitrary config values to be passed
in via the command-line. The argument should be in TOML syntax of KEY=VALUE:

```console
cargo +nightly -Zunstable-options --config net.git-fetch-with-cli=true fetch
```

The `--config` option may be specified multiple times, in which case the
values are merged in left-to-right order, using the same merging logic that
multiple config files use. CLI values take precedence over environment
variables, which take precedence over config files.

Some examples of what it looks like using Bourne shell syntax:

```console
# Most shells will require escaping.
cargo --config http.proxy=\"http://example.com\" …

# Spaces may be used.
cargo --config "net.git-fetch-with-cli = true" …

# TOML array example. Single quotes make it easier to read and write.
cargo --config 'build.rustdocflags = ["--html-in-header", "header.html"]' …

# Example of a complex TOML key.
cargo --config "target.'cfg(all(target_arch = \"arm\", target_os = \"none\"))'.runner = 'my-runner'" …

# Example of overriding a profile setting.
cargo --config profile.dev.package.image.opt-level=3 …
```

### config-include
* Tracking Issue: [#7723](https://github.com/rust-lang/cargo/issues/7723)

The `include` key in a config file can be used to load another config file. It
takes a string for a path to another file relative to the config file, or a
list of strings. It requires the `-Zconfig-include` command-line option.

```toml
# .cargo/config
include = '../../some-common-config.toml'
```

The config values are first loaded from the include path, and then the config
file's own values are merged on top of it.

This can be paired with [config-cli](#config-cli) to specify a file to load
from the command-line. Pass a path to a config file as the argument to
`--config`:

```console
cargo +nightly -Zunstable-options -Zconfig-include --config somefile.toml build
```

CLI paths are relative to the current working directory.

### host-config
* Original Pull Request: [#9322](https://github.com/rust-lang/cargo/pull/9322)
* Tracking Issue: [#3349](https://github.com/rust-lang/cargo/issues/3349)

The `host` key in a config file can be used pass flags to host build targets
such as build scripts that must run on the host system instead of the target
system when cross compiling. It supports both generic and host arch specific
tables. Matching host arch tables take precedence over generic host tables.

It requires the `-Zhost-config` command-line option.

```toml
# .cargo/config
cargo-features = ["host-config"]

[host]
linker = "/path/to/host/linker"
[host.x86_64-unknown-linux-gnu]
linker = "/path/to/host/arch/linker"
[target.x86_64-unknown-linux-gnu]
linker = "/path/to/target/linker"
```

The generic `host` table above will be entirely ignored when building on a
`x86_64-unknown-linux-gnu` host as the `host.x86_64-unknown-linux-gnu` table
takes precedence.

This feature requires a `--target` to be specified.

```console
cargo +nightly -Zunstable-options -Zhost-config --config somefile.toml build --target x86_64-unknown-linux-gnu
```

### unit-graph
* Tracking Issue: [#8002](https://github.com/rust-lang/cargo/issues/8002)

The `--unit-graph` flag can be passed to any build command (`build`, `check`,
`run`, `test`, `bench`, `doc`, etc.) to emit a JSON object to stdout which
represents Cargo's internal unit graph. Nothing is actually built, and the
command returns immediately after printing. Each "unit" corresponds to an
execution of the compiler. These objects also include which unit each unit
depends on.

```
cargo +nightly build --unit-graph -Z unstable-options
```

This structure provides a more complete view of the dependency relationship as
Cargo sees it. In particular, the "features" field supports the new feature
resolver where a dependency can be built multiple times with different
features. `cargo metadata` fundamentally cannot represent the relationship of
features between different dependency kinds, and features now depend on which
command is run and which packages and targets are selected. Additionally it
can provide details about intra-package dependencies like build scripts or
tests.

The following is a description of the JSON structure:

```javascript
{
  /* Version of the JSON output structure. If any backwards incompatible
     changes are made, this value will be increased.
  */
  "version": 1,
  /* Array of all build units. */
  "units": [
    {
      /* An opaque string which indicates the package.
         Information about the package can be obtained from `cargo metadata`.
      */
      "pkg_id": "my-package 0.1.0 (path+file:///path/to/my-package)",
      /* The Cargo target. See the `cargo metadata` documentation for more
         information about these fields.
         https://doc.rust-lang.org/cargo/commands/cargo-metadata.html
      */
      "target": {
        "kind": ["lib"],
        "crate_types": ["lib"],
        "name": "my-package",
        "src_path": "/path/to/my-package/src/lib.rs",
        "edition": "2018",
        "test": true,
        "doctest": true
      },
      /* The profile settings for this unit.
         These values may not match the profile defined in the manifest.
         Units can use modified profile settings. For example, the "panic"
         setting can be overridden for tests to force it to "unwind".
      */
      "profile": {
        /* The profile name these settings are derived from. */
        "name": "dev",
        /* The optimization level as a string. */
        "opt_level": "0",
        /* The LTO setting as a string. */
        "lto": "false",
        /* The codegen units as an integer.
           `null` if it should use the compiler's default.
        */
        "codegen_units": null,
        /* The debug information level as an integer.
           `null` if it should use the compiler's default (0).
        */
        "debuginfo": 2,
        /* Whether or not debug-assertions are enabled. */
        "debug_assertions": true,
        /* Whether or not overflow-checks are enabled. */
        "overflow_checks": true,
        /* Whether or not rpath is enabled. */
        "rpath": false,
        /* Whether or not incremental is enabled. */
        "incremental": true,
        /* The panic strategy, "unwind" or "abort". */
        "panic": "unwind"
      },
      /* Which platform this target is being built for.
         A value of `null` indicates it is for the host.
         Otherwise it is a string of the target triple (such as
         "x86_64-unknown-linux-gnu").
      */
      "platform": null,
      /* The "mode" for this unit. Valid values:

         * "test" — Build using `rustc` as a test.
         * "build" — Build using `rustc`.
         * "check" — Build using `rustc` in "check" mode.
         * "doc" — Build using `rustdoc`.
         * "doctest" — Test using `rustdoc`.
         * "run-custom-build" — Represents the execution of a build script.
      */
      "mode": "build",
      /* Array of features enabled on this unit as strings. */
      "features": ["somefeat"],
      /* Whether or not this is a standard-library unit,
         part of the unstable build-std feature.
         If not set, treat as `false`.
      */
      "is_std": false,
      /* Array of dependencies of this unit. */
      "dependencies": [
        {
          /* Index in the "units" array for the dependency. */
          "index": 1,
          /* The name that this dependency will be referred as. */
          "extern_crate_name": "unicode_xid",
          /* Whether or not this dependency is "public",
             part of the unstable public-dependency feature.
             If not set, the public-dependency feature is not enabled.
          */
          "public": false,
          /* Whether or not this dependency is injected into the prelude,
             currently used by the build-std feature.
             If not set, treat as `false`.
          */
          "noprelude": false
        }
      ]
    },
    // ...
  ],
  /* Array of indices in the "units" array that are the "roots" of the
     dependency graph.
  */
  "roots": [0],
}
```

### Profile `strip` option
* Tracking Issue: [rust-lang/rust#72110](https://github.com/rust-lang/rust/issues/72110)

This feature provides a new option in the `[profile]` section to strip either
symbols or debuginfo from a binary. This can be enabled like so:

```toml
cargo-features = ["strip"]

[package]
# ...

[profile.release]
strip = "debuginfo"
```

Other possible string values of `strip` are `none`, `symbols`, and `off`. The default is `none`.

You can also configure this option with the two absolute boolean values
`true` and `false`. The former enables `strip` at its higher level, `symbols`,
whilst the later disables `strip` completely.

### rustdoc-map
* Tracking Issue: [#8296](https://github.com/rust-lang/cargo/issues/8296)

This feature adds configuration settings that are passed to `rustdoc` so that
it can generate links to dependencies whose documentation is hosted elsewhere
when the dependency is not documented. First, add this to `.cargo/config`:

```toml
[doc.extern-map.registries]
crates-io = "https://docs.rs/"
```

Then, when building documentation, use the following flags to cause links
to dependencies to link to [docs.rs](https://docs.rs/):

```
cargo +nightly doc --no-deps -Zrustdoc-map
```

The `registries` table contains a mapping of registry name to the URL to link
to. The URL may have the markers `{pkg_name}` and `{version}` which will get
replaced with the corresponding values. If neither are specified, then Cargo
defaults to appending `{pkg_name}/{version}/` to the end of the URL.

Another config setting is available to redirect standard library links. By
default, rustdoc creates links to <https://doc.rust-lang.org/nightly/>. To
change this behavior, use the `doc.extern-map.std` setting:

```toml
[doc.extern-map]
std = "local"
```

A value of `"local"` means to link to the documentation found in the `rustc`
sysroot. If you are using rustup, this documentation can be installed with
`rustup component add rust-docs`.

The default value is `"remote"`.

The value may also take a URL for a custom location.

### terminal-width
This feature provides a new flag, `-Z terminal-width`, which is used to pass
a terminal width to `rustc` so that error messages containing long lines
can be intelligently truncated.

For example, passing `-Z terminal-width=20` (an arbitrarily low value) might
produce the following error:

```text
error[E0308]: mismatched types
  --> src/main.rs:2:17
  |
2 | ..._: () = 42;
  |       --   ^^ expected `()`, found integer
  |       |
  |       expected due to this

error: aborting due to previous error
```

In contrast, without `-Z terminal-width`, the error would look as shown below:

```text
error[E0308]: mismatched types
 --> src/main.rs:2:17
  |
2 |     let _: () = 42;
  |            --   ^^ expected `()`, found integer
  |            |
  |            expected due to this

error: aborting due to previous error
```

### Weak dependency features
* Tracking Issue: [#8832](https://github.com/rust-lang/cargo/issues/8832)

The `-Z weak-dep-features` command-line options enables the ability to use
`dep_name?/feat_name` syntax in the `[features]` table. The `?` indicates that
the optional dependency `dep_name` will not be automatically enabled. The
feature `feat_name` will only be added if something else enables the
`dep_name` dependency.

Example:

```toml
[dependencies]
serde = { version = "1.0.117", optional = true, default-features = false }

[features]
std = ["serde?/std"]
```

In this example, the `std` feature enables the `std` feature on the `serde`
dependency. However, unlike the normal `serde/std` syntax, it will not enable
the optional dependency `serde` unless something else has included it.

### per-package-target

The `per-package-target` feature adds two keys to the manifest:
`package.default-target` and `package.forced-target`. The first makes
the package be compiled by default (ie. when no `--target` argument is
passed) for some target. The second one makes the package always be
compiled for the target.

Example:

```toml
[package]
forced-target = "wasm32-unknown-unknown"
```

In this example, the crate is always built for
`wasm32-unknown-unknown`, for instance because it is going to be used
as a plugin for a main program that runs on the host (or provided on
the command line) target.

### credential-process
* Tracking Issue: [#8933](https://github.com/rust-lang/cargo/issues/8933)
* RFC: [#2730](https://github.com/rust-lang/rfcs/pull/2730)

The `credential-process` feature adds a config setting to fetch registry
authentication tokens by calling an external process.

Token authentication is used by the [`cargo login`], [`cargo publish`],
[`cargo owner`], and [`cargo yank`] commands. Additionally, this feature adds
a new `cargo logout` command.

To use this feature, you must pass the `-Z credential-process` flag on the
command-line. Additionally, you must remove any current tokens currently saved
in the [`credentials` file] (which can be done with the new `logout` command).

#### `credential-process` Configuration

To configure which process to run to fetch the token, specify the process in
the `registry` table in a [config file]:

```toml
[registry]
credential-process = "/usr/bin/cargo-creds"
```

If you want to use a different process for a specific registry, it can be
specified in the `registries` table:

```toml
[registries.my-registry]
credential-process = "/usr/bin/cargo-creds"
```

The value can be a string with spaces separating arguments or it can be a TOML
array of strings.

Command-line arguments allow special placeholders which will be replaced with
the corresponding value:

* `{name}` — The name of the registry.
* `{api_url}` — The base URL of the registry API endpoints.
* `{action}` — The authentication action (described below).

Process names with the prefix `cargo:` are loaded from the `libexec` directory
next to cargo. Several experimental credential wrappers are included with
Cargo, and this provides convenient access to them:

```toml
[registry]
credential-process = "cargo:macos-keychain"
```

The current wrappers are:

* `cargo:macos-keychain`: Uses the macOS Keychain to store the token.
* `cargo:wincred`: Uses the Windows Credential Manager to store the token.
* `cargo:1password`: Uses the 1password `op` CLI to store the token. You must
  install the `op` CLI from the [1password
  website](https://1password.com/downloads/command-line/). You must run `op
  signin` at least once with the appropriate arguments (such as `op signin
  my.1password.com user@example.com`), unless you provide the sign-in-address
  and email arguments. The master password will be required on each request
  unless the appropriate `OP_SESSION` environment variable is set. It supports
  the following command-line arguments:
  * `--account`: The account shorthand name to use.
  * `--vault`: The vault name to use.
  * `--sign-in-address`: The sign-in-address, which is a web address such as `my.1password.com`.
  * `--email`: The email address to sign in with.

A wrapper is available for GNOME
[libsecret](https://wiki.gnome.org/Projects/Libsecret) to store tokens on
Linux systems. Due to build limitations, this wrapper is not available as a
pre-compiled binary. This can be built and installed manually. First, install
libsecret using your system package manager (for example, `sudo apt install
libsecret-1-dev`). Then build and install the wrapper with `cargo install
cargo-credential-gnome-secret`.
In the config, use a path to the binary like this:

```toml
[registry]
credential-process = "cargo-credential-gnome-secret {action}"
```

#### `credential-process` Interface

There are two different kinds of token processes that Cargo supports. The
simple "basic" kind will only be called by Cargo when it needs a token. This
is intended for simple and easy integration with password managers, that can
often use pre-existing tooling. The more advanced "Cargo" kind supports
different actions passed as a command-line argument. This is intended for more
pleasant integration experience, at the expense of requiring a Cargo-specific
process to glue to the password manager. Cargo will determine which kind is
supported by the `credential-process` definition. If it contains the
`{action}` argument, then it uses the advanced style, otherwise it assumes it
only supports the "basic" kind.

##### Basic authenticator

A basic authenticator is a process that returns a token on stdout. Newlines
will be trimmed. The process inherits the user's stdin and stderr. It should
exit 0 on success, and nonzero on error.

With this form, [`cargo login`] and `cargo logout` are not supported and
return an error if used.

##### Cargo authenticator

The protocol between the Cargo and the process is very basic, intended to
ensure the credential process is kept as simple as possible. Cargo will
execute the process with the `{action}` argument indicating which action to
perform:

* `store` — Store the given token in secure storage.
* `get` — Get a token from storage.
* `erase` — Remove a token from storage.

The `cargo login` command uses `store` to save a token. Commands that require
authentication, like `cargo publish`, uses `get` to retrieve a token. `cargo
logout` uses the `erase` command to remove a token.

The process inherits the user's stderr, so the process can display messages.
Some values are passed in via environment variables (see below). The expected
interactions are:

* `store` — The token is sent to the process's stdin, terminated by a newline.
  The process should store the token keyed off the registry name. If the
  process fails, it should exit with a nonzero exit status.

* `get` — The process should send the token to its stdout (trailing newline
  will be trimmed). The process inherits the user's stdin, should it need to
  receive input.

  If the process is unable to fulfill the request, it should exit with a
  nonzero exit code.

* `erase` — The process should remove the token associated with the registry
  name. If the token is not found, the process should exit with a 0 exit
  status.

##### Environment

The following environment variables will be provided to the executed command:

* `CARGO` — Path to the `cargo` binary executing the command.
* `CARGO_REGISTRY_NAME` — Name of the registry the authentication token is for.
* `CARGO_REGISTRY_API_URL` — The URL of the registry API.

#### `cargo logout`

A new `cargo logout` command has been added to make it easier to remove a
token from storage. This supports both [`credentials` file] tokens and
`credential-process` tokens.

When used with `credentials` file tokens, it needs the `-Z unstable-options`
command-line option:

```console
cargo logout -Z unstable-options
```

When used with the `credential-process` config, use the `-Z
credential-process` command-line option:


```console
cargo logout -Z credential-process
```

[`cargo login`]: ../commands/cargo-login.md
[`cargo publish`]: ../commands/cargo-publish.md
[`cargo owner`]: ../commands/cargo-owner.md
[`cargo yank`]: ../commands/cargo-yank.md
[`credentials` file]: config.md#credentials
[crates.io]: https://crates.io/
[config file]: config.md

### rust-version
* RFC: [#2495](https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md)
* rustc Tracking Issue: [#65262](https://github.com/rust-lang/rust/issues/65262)

The `-Z rust-version` flag enables the reading the `rust-version` field in the
Cargo manifest `package` section. This can be used by a package to state a minimal
version of the compiler required to build the package. An error is generated if
the version of rustc is older than the stated `rust-version`. The
`--ignore-rust-version` flag can be used to override the check.

```toml
cargo-features = ["rust-version"]

[package]
name = "mypackage"
version = "0.0.1"
rust-version = "1.42"
```

### edition 2021

Support for the 2021 [edition] can be enabled by adding the `edition2021`
unstable feature to the top of `Cargo.toml`:

```toml
cargo-features = ["edition2021"]

[package]
name = "my-package"
version = "0.1.0"
edition = "2021"
```

If you want to transition an existing project from a previous edition, then
`cargo fix --edition` can be used on the nightly channel. After running `cargo
fix`, you can switch the edition to 2021 as illustrated above.

This feature is very unstable, and is only intended for early testing and
experimentation. Future nightly releases may introduce changes for the 2021
edition that may break your build.

The 2021 edition will set the default [resolver version] to "2".

[edition]: ../../edition-guide/index.html
[resolver version]: resolver.md#resolver-versions

### future incompat report
* RFC: [#2834](https://github.com/rust-lang/rfcs/blob/master/text/2834-cargo-report-future-incompat.md)
* rustc Tracking Issue: [#71249](https://github.com/rust-lang/rust/issues/71249)

The `-Z future-incompat-report` flag enables the creation of a future-incompat report
for all dependencies. This makes users aware if any of their crate's dependencies
might stop compiling with a future version of Rust.

### configurable-env
* Original Pull Request: [#9175](https://github.com/rust-lang/cargo/pull/9175)

The `-Z configurable-env` flag enables the `[env]` section in the
`.cargo/config.toml` file. This section allows you to set additional environment
variables for build scripts, rustc invocations, `cargo run` and `cargo build`.

```toml
[env]
OPENSSL_DIR = "/opt/openssl"
```

By default, the variables specified will not override values that already exist
in the environment. This behavior can be changed by setting the `force` flag.

Setting the `relative` flag evaluates the value as a config-relative path that
is relative to the parent directory of the `.cargo` directory that contains the
`config.toml` file. The value of the environment variable will be the full
absolute path.

```toml
[env]
TMPDIR = { value = "/home/tmp", force = true }
OPENSSL_DIR = { value = "vendor/openssl", relative = true }
```

### patch-in-config
* Original Pull Request: [#9204](https://github.com/rust-lang/cargo/pull/9204)
* Tracking Issue: [#9269](https://github.com/rust-lang/cargo/issues/9269)

The `-Z patch-in-config` flag enables the use of `[patch]` sections in
cargo configuration files (`.cargo/config.toml`). The format of such
`[patch]` sections is identical to the one used in `Cargo.toml`.

Since `.cargo/config.toml` files are not usually checked into source
control, you should prefer patching using `Cargo.toml` where possible to
ensure that other developers can compile your crate in their own
environments. Patching through cargo configuration files is generally
only appropriate when the patch section is automatically generated by an
external build tool.

If a given dependency is patched both in a cargo configuration file and
a `Cargo.toml` file, the patch in `Cargo.toml` is used. If multiple
configuration files patch the same dependency, standard cargo
configuration merging is used, which prefers the value defined closest
to the current directory, with `$HOME/.cargo/config.toml` taking the
lowest precedence.

Relative `path` dependencies in such a `[patch]` section are resolved
relative to the configuration file they appear in.

## `cargo config`

* Original Issue: [#2362](https://github.com/rust-lang/cargo/issues/2362)
* Tracking Issue: [#9301](https://github.com/rust-lang/cargo/issues/9301)

The `cargo config` subcommand provides a way to display the configuration
files that cargo loads. It currently includes the `get` subcommand which
can take an optional config value to display.

```console
cargo +nightly -Zunstable-options config get build.rustflags
```

If no config value is included, it will display all config values. See the
`--help` output for more options available.


<script>
(function() {
    var fragments = {
        "#edition": "manifest.html#the-edition-field",
        "#compile-progress": "config.html#termprogresswhen",
        "#rename-dependency": "specifying-dependencies.html#renaming-dependencies-in-cargotoml",
        "#alternate-registries": "registries.html",
        "#offline-mode": "../commands/cargo.html",
        "#publish-lockfile": "../commands/cargo-package.html",
        "#default-run": "manifest.html#the-default-run-field",
        "#cache-messages": "https://github.com/rust-lang/cargo/pull/7450",
        "#install-upgrade": "../commands/cargo-install.html",
        "#profile-overrides": "profiles.html#overrides",
        "#config-profiles": "config.html#profile",
        "#crate-versions": "https://github.com/rust-lang/cargo/pull/8509",
        "#features": "features.html#feature-resolver-version-2",
        "#package-features": "features.html#resolver-version-2-command-line-flags",
        "#resolver": "resolver.html#resolver-versions",
    };
    var target = fragments[window.location.hash];
    if (target) {
        if (target.startsWith('https')) {
          window.location.replace(target);
        } else {
          var url = window.location.toString();
          var base = url.substring(0, url.lastIndexOf('/'));
          window.location.replace(base + "/" + target);
        }
    }
})();
</script>
