use std::collections::HashMap;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};

use cargo::sources::CRATES_IO_INDEX;
use cargo::util::Sha256;
use flate2::write::GzEncoder;
use flate2::Compression;
use git2;
use hex;
use tar::{Builder, Header};
use url::Url;

use crate::support::git::repo;
use crate::support::paths;

pub fn registry_path() -> PathBuf {
    paths::root().join("registry")
}
pub fn registry() -> Url {
    Url::from_file_path(&*registry_path()).ok().unwrap()
}
pub fn api_path() -> PathBuf {
    paths::root().join("api")
}
pub fn dl_path() -> PathBuf {
    paths::root().join("dl")
}
pub fn dl_url() -> Url {
    Url::from_file_path(&*dl_path()).ok().unwrap()
}
pub fn alt_registry_path() -> PathBuf {
    paths::root().join("alternative-registry")
}
pub fn alt_registry() -> Url {
    Url::from_file_path(&*alt_registry_path()).ok().unwrap()
}
pub fn alt_dl_path() -> PathBuf {
    paths::root().join("alt_dl")
}
pub fn alt_dl_url() -> String {
    let base = Url::from_file_path(&*alt_dl_path()).ok().unwrap();
    format!("{}/{{crate}}/{{version}}/{{crate}}-{{version}}.crate", base)
}
pub fn alt_api_path() -> PathBuf {
    paths::root().join("alt_api")
}
pub fn alt_api_url() -> Url {
    Url::from_file_path(&*alt_api_path()).ok().unwrap()
}

/// A builder for creating a new package in a registry.
///
/// This uses "source replacement" using an automatically generated
/// `.cargo/config` file to ensure that dependencies will use these packages
/// instead of contacting crates.io. See `source-replacement.md` for more
/// details on how source replacement works.
///
/// Call `publish` to finalize and create the package.
///
/// If no files are specified, an empty `lib.rs` file is automatically created.
///
/// The `Cargo.toml` file is automatically generated based on the methods
/// called on `Package` (for example, calling `dep()` will add to the
/// `[dependencies]` automatically). You may also specify a `Cargo.toml` file
/// to override the generated one.
///
/// This supports different registry types:
/// - Regular source replacement that replaces `crates.io` (the default).
/// - A "local registry" which is a subset for vendoring (see
///   `Package::local`).
/// - An "alternative registry" which requires specifying the registry name
///   (see `Package::alternative`).
///
/// This does not support "directory sources". See `directory.rs` for
/// `VendorPackage` which implements directory sources.
///
/// # Example
/// ```
/// // Publish package "a" depending on "b".
/// Package::new("a", "1.0.0")
///     .dep("b", "1.0.0")
///     .file("src/lib.rs", r#"
///         extern crate b;
///         pub fn f() -> i32 { b::f() * 2 }
///     "#)
///     .publish();
///
/// // Publish package "b".
/// Package::new("b", "1.0.0")
///     .file("src/lib.rs", r#"
///         pub fn f() -> i32 { 12 }
///     "#)
///     .publish();
///
/// // Create a project that uses package "a".
/// let p = project()
///     .file("Cargo.toml", r#"
///         [package]
///         name = "foo"
///         version = "0.0.1"
///
///         [dependencies]
///         a = "1.0"
///     "#)
///     .file("src/main.rs", r#"
///         extern crate a;
///         fn main() { println!("{}", a::f()); }
///     "#)
///     .build();
///
/// p.cargo("run").with_stdout("24").run();
/// ```
pub struct Package {
    name: String,
    vers: String,
    deps: Vec<Dependency>,
    files: Vec<(String, String)>,
    extra_files: Vec<(String, String)>,
    yanked: bool,
    features: HashMap<String, Vec<String>>,
    local: bool,
    alternative: bool,
}

#[derive(Clone)]
pub struct Dependency {
    name: String,
    vers: String,
    kind: String,
    target: Option<String>,
    features: Vec<String>,
    registry: Option<String>,
    package: Option<String>,
    optional: bool,
}

pub fn init() {
    let config = paths::home().join(".cargo/config");
    t!(fs::create_dir_all(config.parent().unwrap()));
    if fs::metadata(&config).is_ok() {
        return;
    }
    t!(t!(File::create(&config)).write_all(
        format!(
            r#"
        [registry]
        token = "api-token"

        [source.crates-io]
        registry = 'https://wut'
        replace-with = 'dummy-registry'

        [source.dummy-registry]
        registry = '{reg}'

        [registries.alternative]
        index = '{alt}'
    "#,
            reg = registry(),
            alt = alt_registry()
        )
        .as_bytes()
    ));

    // Init a new registry
    let _ = repo(&registry_path())
        .file(
            "config.json",
            &format!(
                r#"
            {{"dl":"{0}","api":"{0}"}}
        "#,
                dl_url()
            ),
        )
        .build();
    fs::create_dir_all(dl_path().join("api/v1/crates")).unwrap();

    // Init an alt registry
    repo(&alt_registry_path())
        .file(
            "config.json",
            &format!(
                r#"
            {{"dl":"{}","api":"{}"}}
        "#,
                alt_dl_url(),
                alt_api_url()
            ),
        )
        .build();
    fs::create_dir_all(alt_api_path().join("api/v1/crates")).unwrap();
}

impl Package {
    /// Create a new package builder.
    /// Call `publish()` to finalize and build the package.
    pub fn new(name: &str, vers: &str) -> Package {
        init();
        Package {
            name: name.to_string(),
            vers: vers.to_string(),
            deps: Vec::new(),
            files: Vec::new(),
            extra_files: Vec::new(),
            yanked: false,
            features: HashMap::new(),
            local: false,
            alternative: false,
        }
    }

    /// Call with `true` to publish in a "local registry".
    ///
    /// See `source-replacement.html#local-registry-sources` for more details
    /// on local registries. See `local_registry.rs` for the tests that use
    /// this.
    pub fn local(&mut self, local: bool) -> &mut Package {
        self.local = local;
        self
    }

    /// Call with `true` to publish in an "alternative registry".
    ///
    /// The name of the alternative registry is called "alternative".
    ///
    /// See `unstable.html#alternate-registries` for more details on
    /// alternative registries. See `alt_registry.rs` for the tests that use
    /// this.
    pub fn alternative(&mut self, alternative: bool) -> &mut Package {
        self.alternative = alternative;
        self
    }

    /// Add a file to the package.
    pub fn file(&mut self, name: &str, contents: &str) -> &mut Package {
        self.files.push((name.to_string(), contents.to_string()));
        self
    }

    /// Add an "extra" file that is not rooted within the package.
    ///
    /// Normal files are automatically placed within a directory named
    /// `$PACKAGE-$VERSION`. This allows you to override that behavior,
    /// typically for testing invalid behavior.
    pub fn extra_file(&mut self, name: &str, contents: &str) -> &mut Package {
        self.extra_files
            .push((name.to_string(), contents.to_string()));
        self
    }

    /// Add a normal dependency. Example:
    /// ```
    /// [dependencies]
    /// foo = {version = "1.0"}
    /// ```
    pub fn dep(&mut self, name: &str, vers: &str) -> &mut Package {
        self.add_dep(&Dependency::new(name, vers))
    }

    /// Add a dependency with the given feature. Example:
    /// ```
    /// [dependencies]
    /// foo = {version = "1.0", "features": ["feat1", "feat2"]}
    /// ```
    pub fn feature_dep(&mut self, name: &str, vers: &str, features: &[&str]) -> &mut Package {
        self.add_dep(Dependency::new(name, vers).enable_features(features))
    }

    /// Add a platform-specific dependency. Example:
    /// ```
    /// [target.'cfg(windows)'.dependencies]
    /// foo = {version = "1.0"}
    /// ```
    pub fn target_dep(&mut self, name: &str, vers: &str, target: &str) -> &mut Package {
        self.add_dep(Dependency::new(name, vers).target(target))
    }

    /// Add a dependency to the alternative registry.
    pub fn registry_dep(&mut self, name: &str, vers: &str) -> &mut Package {
        self.add_dep(Dependency::new(name, vers).registry("alternative"))
    }

    /// Add a dev-dependency. Example:
    /// ```
    /// [dev-dependencies]
    /// foo = {version = "1.0"}
    /// ```
    pub fn dev_dep(&mut self, name: &str, vers: &str) -> &mut Package {
        self.add_dep(Dependency::new(name, vers).dev())
    }

    /// Add a build-dependency. Example:
    /// ```
    /// [build-dependencies]
    /// foo = {version = "1.0"}
    /// ```
    pub fn build_dep(&mut self, name: &str, vers: &str) -> &mut Package {
        self.add_dep(Dependency::new(name, vers).build())
    }

    pub fn add_dep(&mut self, dep: &Dependency) -> &mut Package {
        self.deps.push(dep.clone());
        self
    }

    /// Specify whether or not the package is "yanked".
    pub fn yanked(&mut self, yanked: bool) -> &mut Package {
        self.yanked = yanked;
        self
    }

    /// Add an entry in the `[features]` section
    pub fn feature(&mut self, name: &str, deps: &[&str]) -> &mut Package {
        let deps = deps.iter().map(|s| s.to_string()).collect();
        self.features.insert(name.to_string(), deps);
        self
    }

    /// Create the package and place it in the registry.
    ///
    /// This does not actually use Cargo's publishing system, but instead
    /// manually creates the entry in the registry on the filesystem.
    ///
    /// Returns the checksum for the package.
    pub fn publish(&self) -> String {
        self.make_archive();

        // Figure out what we're going to write into the index
        let deps = self
            .deps
            .iter()
            .map(|dep| {
                // In the index, the `registry` is null if it is from the same registry.
                // In Cargo.toml, it is None if it is from crates.io.
                let registry_url =
                    match (self.alternative, dep.registry.as_ref().map(|s| s.as_ref())) {
                        (false, None) => None,
                        (false, Some("alternative")) => Some(alt_registry().to_string()),
                        (true, None) => Some(CRATES_IO_INDEX.to_string()),
                        (true, Some("alternative")) => None,
                        _ => panic!("registry_dep currently only supports `alternative`"),
                    };
                serde_json::json!({
                    "name": dep.name,
                    "req": dep.vers,
                    "features": dep.features,
                    "default_features": true,
                    "target": dep.target,
                    "optional": dep.optional,
                    "kind": dep.kind,
                    "registry": registry_url,
                    "package": dep.package,
                })
            })
            .collect::<Vec<_>>();
        let cksum = {
            let mut c = Vec::new();
            t!(t!(File::open(&self.archive_dst())).read_to_end(&mut c));
            cksum(&c)
        };
        let line = serde_json::json!({
            "name": self.name,
            "vers": self.vers,
            "deps": deps,
            "cksum": cksum,
            "features": self.features,
            "yanked": self.yanked,
        })
        .to_string();

        let file = match self.name.len() {
            1 => format!("1/{}", self.name),
            2 => format!("2/{}", self.name),
            3 => format!("3/{}/{}", &self.name[..1], self.name),
            _ => format!("{}/{}/{}", &self.name[0..2], &self.name[2..4], self.name),
        };

        let registry_path = if self.alternative {
            alt_registry_path()
        } else {
            registry_path()
        };

        // Write file/line in the index
        let dst = if self.local {
            registry_path.join("index").join(&file)
        } else {
            registry_path.join(&file)
        };
        let mut prev = String::new();
        let _ = File::open(&dst).and_then(|mut f| f.read_to_string(&mut prev));
        t!(fs::create_dir_all(dst.parent().unwrap()));
        t!(t!(File::create(&dst)).write_all((prev + &line[..] + "\n").as_bytes()));

        // Add the new file to the index
        if !self.local {
            let repo = t!(git2::Repository::open(&registry_path));
            let mut index = t!(repo.index());
            t!(index.add_path(Path::new(&file)));
            t!(index.write());
            let id = t!(index.write_tree());

            // Commit this change
            let tree = t!(repo.find_tree(id));
            let sig = t!(repo.signature());
            let parent = t!(repo.refname_to_id("refs/heads/master"));
            let parent = t!(repo.find_commit(parent));
            t!(repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Another commit",
                &tree,
                &[&parent]
            ));
        }

        cksum
    }

    fn make_archive(&self) {
        let features = if self.deps.iter().any(|dep| dep.registry.is_some()) {
            "cargo-features = [\"alternative-registries\"]\n"
        } else {
            ""
        };
        let mut manifest = format!(
            r#"
            {}[package]
            name = "{}"
            version = "{}"
            authors = []
        "#,
            features, self.name, self.vers
        );
        for dep in self.deps.iter() {
            let target = match dep.target {
                None => String::new(),
                Some(ref s) => format!("target.'{}'.", s),
            };
            let kind = match &dep.kind[..] {
                "build" => "build-",
                "dev" => "dev-",
                _ => "",
            };
            manifest.push_str(&format!(
                r#"
                [{}{}dependencies.{}]
                version = "{}"
            "#,
                target, kind, dep.name, dep.vers
            ));
            if let Some(registry) = &dep.registry {
                assert_eq!(registry, "alternative");
                manifest.push_str(&format!("registry-index = \"{}\"", alt_registry()));
            }
        }

        let dst = self.archive_dst();
        t!(fs::create_dir_all(dst.parent().unwrap()));
        let f = t!(File::create(&dst));
        let mut a = Builder::new(GzEncoder::new(f, Compression::default()));
        self.append(&mut a, "Cargo.toml", &manifest);
        if self.files.is_empty() {
            self.append(&mut a, "src/lib.rs", "");
        } else {
            for &(ref name, ref contents) in self.files.iter() {
                self.append(&mut a, name, contents);
            }
        }
        for &(ref name, ref contents) in self.extra_files.iter() {
            self.append_extra(&mut a, name, contents);
        }
    }

    fn append<W: Write>(&self, ar: &mut Builder<W>, file: &str, contents: &str) {
        self.append_extra(
            ar,
            &format!("{}-{}/{}", self.name, self.vers, file),
            contents,
        );
    }

    fn append_extra<W: Write>(&self, ar: &mut Builder<W>, path: &str, contents: &str) {
        let mut header = Header::new_ustar();
        header.set_size(contents.len() as u64);
        t!(header.set_path(path));
        header.set_cksum();
        t!(ar.append(&header, contents.as_bytes()));
    }

    /// Returns the path to the compressed package file.
    pub fn archive_dst(&self) -> PathBuf {
        if self.local {
            registry_path().join(format!("{}-{}.crate", self.name, self.vers))
        } else if self.alternative {
            alt_dl_path()
                .join(&self.name)
                .join(&self.vers)
                .join(&format!("{}-{}.crate", self.name, self.vers))
        } else {
            dl_path().join(&self.name).join(&self.vers).join("download")
        }
    }
}

pub fn cksum(s: &[u8]) -> String {
    let mut sha = Sha256::new();
    sha.update(s);
    hex::encode(&sha.finish())
}

impl Dependency {
    pub fn new(name: &str, vers: &str) -> Dependency {
        Dependency {
            name: name.to_string(),
            vers: vers.to_string(),
            kind: "normal".to_string(),
            target: None,
            features: Vec::new(),
            package: None,
            optional: false,
            registry: None,
        }
    }

    /// Change this to `[build-dependencies]`
    pub fn build(&mut self) -> &mut Self {
        self.kind = "build".to_string();
        self
    }

    /// Change this to `[dev-dependencies]`
    pub fn dev(&mut self) -> &mut Self {
        self.kind = "dev".to_string();
        self
    }

    /// Change this to `[target.$target.dependencies]`
    pub fn target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_string());
        self
    }

    /// Add `registry = $registry` to this dependency
    pub fn registry(&mut self, registry: &str) -> &mut Self {
        self.registry = Some(registry.to_string());
        self
    }

    /// Add `features = [ ... ]` to this dependency
    pub fn enable_features(&mut self, features: &[&str]) -> &mut Self {
        self.features.extend(features.iter().map(|s| s.to_string()));
        self
    }

    /// Add `package = ...` to this dependency
    pub fn package(&mut self, pkg: &str) -> &mut Self {
        self.package = Some(pkg.to_string());
        self
    }

    /// Change this to an optional dependency
    pub fn optional(&mut self, optional: bool) -> &mut Self {
        self.optional = optional;
        self
    }
}
