use std::cmp::{self, Ordering};
use std::collections::HashSet;
use std::fmt::{self, Formatter};
use std::hash::{self, Hash};
use std::path::Path;
use std::ptr;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, ATOMIC_BOOL_INIT};
use std::sync::Mutex;

use log::trace;
use serde::de;
use serde::ser;
use url::Url;

use crate::ops;
use crate::sources::git;
use crate::sources::DirectorySource;
use crate::sources::{GitSource, PathSource, RegistrySource, CRATES_IO_INDEX};
use crate::util::{CargoResult, Config, ToUrl};

lazy_static::lazy_static! {
    static ref SOURCE_ID_CACHE: Mutex<HashSet<&'static SourceIdInner>> = Mutex::new(HashSet::new());
}

/// Unique identifier for a source of packages.
#[derive(Clone, Copy, Eq, Debug)]
pub struct SourceId {
    inner: &'static SourceIdInner,
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
struct SourceIdInner {
    /// The source URL
    url: Url,
    /// `git::canonicalize_url(url)` for the url field
    canonical_url: Url,
    /// The source kind
    kind: Kind,
    // e.g. the exact git revision of the specified branch for a Git Source
    precise: Option<String>,
    /// Name of the registry source for alternative registries
    name: Option<String>,
}

/// The possible kinds of code source. Along with SourceIdInner this fully defines the
/// source
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    /// Kind::Git(<git reference>) represents a git repository
    Git(GitReference),
    /// represents a local path
    Path,
    /// represents a remote registry
    Registry,
    /// represents a local filesystem-based registry
    LocalRegistry,
    /// represents a directory-based registry
    Directory,
}

/// Information to find a specific commit in a git repository
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitReference {
    /// from a tag
    Tag(String),
    /// from the HEAD of a branch
    Branch(String),
    /// from a specific revision
    Rev(String),
}

impl SourceId {
    /// Create a SourceId object from the kind and url.
    ///
    /// The canonical url will be calculated, but the precise field will not
    fn new(kind: Kind, url: Url) -> CargoResult<SourceId> {
        let source_id = SourceId::wrap(SourceIdInner {
            kind,
            canonical_url: git::canonicalize_url(&url)?,
            url,
            precise: None,
            name: None,
        });
        Ok(source_id)
    }

    fn wrap(inner: SourceIdInner) -> SourceId {
        let mut cache = SOURCE_ID_CACHE.lock().unwrap();
        let inner = cache.get(&inner).cloned().unwrap_or_else(|| {
            let inner = Box::leak(Box::new(inner));
            cache.insert(inner);
            inner
        });
        SourceId { inner }
    }

    /// Parses a source URL and returns the corresponding ID.
    ///
    /// ## Example
    ///
    /// ```
    /// use cargo::core::SourceId;
    /// SourceId::from_url("git+https://github.com/alexcrichton/\
    ///                     libssh2-static-sys#80e71a3021618eb05\
    ///                     656c58fb7c5ef5f12bc747f");
    /// ```
    pub fn from_url(string: &str) -> CargoResult<SourceId> {
        let mut parts = string.splitn(2, '+');
        let kind = parts.next().unwrap();
        let url = parts
            .next()
            .ok_or_else(|| format_err!("invalid source `{}`", string))?;

        match kind {
            "git" => {
                let mut url = url.to_url()?;
                let mut reference = GitReference::Branch("master".to_string());
                for (k, v) in url.query_pairs() {
                    match &k[..] {
                        // map older 'ref' to branch
                        "branch" | "ref" => reference = GitReference::Branch(v.into_owned()),

                        "rev" => reference = GitReference::Rev(v.into_owned()),
                        "tag" => reference = GitReference::Tag(v.into_owned()),
                        _ => {}
                    }
                }
                let precise = url.fragment().map(|s| s.to_owned());
                url.set_fragment(None);
                url.set_query(None);
                Ok(SourceId::for_git(&url, reference)?.with_precise(precise))
            }
            "registry" => {
                let url = url.to_url()?;
                Ok(SourceId::new(Kind::Registry, url)?.with_precise(Some("locked".to_string())))
            }
            "path" => {
                let url = url.to_url()?;
                SourceId::new(Kind::Path, url)
            }
            kind => Err(format_err!("unsupported source protocol: {}", kind)),
        }
    }

    /// A view of the `SourceId` that can be `Display`ed as a URL
    pub fn to_url(&self) -> SourceIdToUrl<'_> {
        SourceIdToUrl {
            inner: &*self.inner,
        }
    }

    /// Create a SourceId from a filesystem path.
    ///
    /// Pass absolute path
    pub fn for_path(path: &Path) -> CargoResult<SourceId> {
        let url = path.to_url()?;
        SourceId::new(Kind::Path, url)
    }

    /// Crate a SourceId from a git reference
    pub fn for_git(url: &Url, reference: GitReference) -> CargoResult<SourceId> {
        SourceId::new(Kind::Git(reference), url.clone())
    }

    /// Create a SourceId from a registry url
    pub fn for_registry(url: &Url) -> CargoResult<SourceId> {
        SourceId::new(Kind::Registry, url.clone())
    }

    /// Create a SourceId from a local registry path
    pub fn for_local_registry(path: &Path) -> CargoResult<SourceId> {
        let url = path.to_url()?;
        SourceId::new(Kind::LocalRegistry, url)
    }

    /// Create a SourceId from a directory path
    pub fn for_directory(path: &Path) -> CargoResult<SourceId> {
        let url = path.to_url()?;
        SourceId::new(Kind::Directory, url)
    }

    /// Returns the `SourceId` corresponding to the main repository.
    ///
    /// This is the main cargo registry by default, but it can be overridden in
    /// a `.cargo/config`.
    pub fn crates_io(config: &Config) -> CargoResult<SourceId> {
        config.crates_io_source_id(|| {
            let cfg = ops::registry_configuration(config, None)?;
            let url = if let Some(ref index) = cfg.index {
                static WARNED: AtomicBool = ATOMIC_BOOL_INIT;
                if !WARNED.swap(true, SeqCst) {
                    config.shell().warn(
                        "custom registry support via \
                         the `registry.index` configuration is \
                         being removed, this functionality \
                         will not work in the future",
                    )?;
                }
                &index[..]
            } else {
                CRATES_IO_INDEX
            };
            let url = url.to_url()?;
            SourceId::for_registry(&url)
        })
    }

    pub fn alt_registry(config: &Config, key: &str) -> CargoResult<SourceId> {
        let url = config.get_registry_index(key)?;
        Ok(SourceId::wrap(SourceIdInner {
            kind: Kind::Registry,
            canonical_url: git::canonicalize_url(&url)?,
            url,
            precise: None,
            name: Some(key.to_string()),
        }))
    }

    /// Get this source URL
    pub fn url(&self) -> &Url {
        &self.inner.url
    }

    pub fn display_registry(self) -> String {
        if self.is_default_registry() {
            "crates.io index".to_string()
        } else {
            format!("`{}` index", url_display(self.url()))
        }
    }

    /// Is this source from a filesystem path
    pub fn is_path(self) -> bool {
        self.inner.kind == Kind::Path
    }

    /// Is this source from a registry (either local or not)
    pub fn is_registry(self) -> bool {
        match self.inner.kind {
            Kind::Registry | Kind::LocalRegistry => true,
            _ => false,
        }
    }

    /// Is this source from an alternative registry
    pub fn is_alt_registry(self) -> bool {
        self.is_registry() && self.inner.name.is_some()
    }

    /// Is this source from a git repository
    pub fn is_git(self) -> bool {
        match self.inner.kind {
            Kind::Git(_) => true,
            _ => false,
        }
    }

    /// Creates an implementation of `Source` corresponding to this ID.
    pub fn load<'a>(self, config: &'a Config) -> CargoResult<Box<dyn super::Source + 'a>> {
        trace!("loading SourceId; {}", self);
        match self.inner.kind {
            Kind::Git(..) => Ok(Box::new(GitSource::new(self, config)?)),
            Kind::Path => {
                let path = match self.inner.url.to_file_path() {
                    Ok(p) => p,
                    Err(()) => panic!("path sources cannot be remote"),
                };
                Ok(Box::new(PathSource::new(&path, self, config)))
            }
            Kind::Registry => Ok(Box::new(RegistrySource::remote(self, config))),
            Kind::LocalRegistry => {
                let path = match self.inner.url.to_file_path() {
                    Ok(p) => p,
                    Err(()) => panic!("path sources cannot be remote"),
                };
                Ok(Box::new(RegistrySource::local(self, &path, config)))
            }
            Kind::Directory => {
                let path = match self.inner.url.to_file_path() {
                    Ok(p) => p,
                    Err(()) => panic!("path sources cannot be remote"),
                };
                Ok(Box::new(DirectorySource::new(&path, self, config)))
            }
        }
    }

    /// Get the value of the precise field
    pub fn precise(self) -> Option<&'static str> {
        self.inner.precise.as_ref().map(|s| &s[..])
    }

    /// Get the git reference if this is a git source, otherwise None.
    pub fn git_reference(self) -> Option<&'static GitReference> {
        match self.inner.kind {
            Kind::Git(ref s) => Some(s),
            _ => None,
        }
    }

    /// Create a new SourceId from this source with the given `precise`
    pub fn with_precise(self, v: Option<String>) -> SourceId {
        SourceId::wrap(SourceIdInner {
            precise: v,
            ..(*self.inner).clone()
        })
    }

    /// Whether the remote registry is the standard https://crates.io
    pub fn is_default_registry(self) -> bool {
        match self.inner.kind {
            Kind::Registry => {}
            _ => return false,
        }
        self.inner.url.to_string() == CRATES_IO_INDEX
    }

    /// Hash `self`
    ///
    /// For paths, remove the workspace prefix so the same source will give the
    /// same hash in different locations.
    pub fn stable_hash<S: hash::Hasher>(self, workspace: &Path, into: &mut S) {
        if self.is_path() {
            if let Ok(p) = self
                .inner
                .url
                .to_file_path()
                .unwrap()
                .strip_prefix(workspace)
            {
                self.inner.kind.hash(into);
                p.to_str().unwrap().hash(into);
                return;
            }
        }
        self.hash(into)
    }

    pub fn full_eq(self, other: SourceId) -> bool {
        ptr::eq(self.inner, other.inner)
    }

    pub fn full_hash<S: hash::Hasher>(self, into: &mut S) {
        ptr::NonNull::from(self.inner).hash(into)
    }
}

impl PartialOrd for SourceId {
    fn partial_cmp(&self, other: &SourceId) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourceId {
    fn cmp(&self, other: &SourceId) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl ser::Serialize for SourceId {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        if self.is_path() {
            None::<String>.serialize(s)
        } else {
            s.collect_str(&self.to_url())
        }
    }
}

impl<'de> de::Deserialize<'de> for SourceId {
    fn deserialize<D>(d: D) -> Result<SourceId, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let string = String::deserialize(d)?;
        SourceId::from_url(&string).map_err(de::Error::custom)
    }
}

fn url_display(url: &Url) -> String {
    if url.scheme() == "file" {
        if let Ok(path) = url.to_file_path() {
            if let Some(path_str) = path.to_str() {
                return path_str.to_string();
            }
        }
    }

    url.as_str().to_string()
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self.inner {
            SourceIdInner {
                kind: Kind::Path,
                ref url,
                ..
            } => write!(f, "{}", url_display(url)),
            SourceIdInner {
                kind: Kind::Git(ref reference),
                ref url,
                ref precise,
                ..
            } => {
                // Don't replace the URL display for git references,
                // because those are kind of expected to be URLs.
                write!(f, "{}", url)?;
                if let Some(pretty) = reference.pretty_ref() {
                    write!(f, "?{}", pretty)?;
                }

                if let Some(ref s) = *precise {
                    let len = cmp::min(s.len(), 8);
                    write!(f, "#{}", &s[..len])?;
                }
                Ok(())
            }
            SourceIdInner {
                kind: Kind::Registry,
                ref url,
                ..
            }
            | SourceIdInner {
                kind: Kind::LocalRegistry,
                ref url,
                ..
            } => write!(f, "registry `{}`", url_display(url)),
            SourceIdInner {
                kind: Kind::Directory,
                ref url,
                ..
            } => write!(f, "dir {}", url_display(url)),
        }
    }
}

// Custom equality defined as canonical URL equality for git sources and
// URL equality for other sources, ignoring the `precise` and `name` fields.
impl PartialEq for SourceId {
    fn eq(&self, other: &SourceId) -> bool {
        if ptr::eq(self.inner, other.inner) {
            return true;
        }
        if self.inner.kind != other.inner.kind {
            return false;
        }
        if self.inner.url == other.inner.url {
            return true;
        }

        match (&self.inner.kind, &other.inner.kind) {
            (Kind::Git(ref1), Kind::Git(ref2)) => {
                ref1 == ref2 && self.inner.canonical_url == other.inner.canonical_url
            }
            _ => false,
        }
    }
}

impl PartialOrd for SourceIdInner {
    fn partial_cmp(&self, other: &SourceIdInner) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourceIdInner {
    fn cmp(&self, other: &SourceIdInner) -> Ordering {
        match self.kind.cmp(&other.kind) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.url.cmp(&other.url) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match (&self.kind, &other.kind) {
            (&Kind::Git(ref ref1), &Kind::Git(ref ref2)) => {
                (ref1, &self.canonical_url).cmp(&(ref2, &other.canonical_url))
            }
            _ => self.kind.cmp(&other.kind),
        }
    }
}

// The hash of SourceId is used in the name of some Cargo folders, so shouldn't
// vary. `as_str` gives the serialisation of a url (which has a spec) and so
// insulates against possible changes in how the url crate does hashing.
impl Hash for SourceId {
    fn hash<S: hash::Hasher>(&self, into: &mut S) {
        self.inner.kind.hash(into);
        match *self.inner {
            SourceIdInner {
                kind: Kind::Git(..),
                ref canonical_url,
                ..
            } => canonical_url.as_str().hash(into),
            _ => self.inner.url.as_str().hash(into),
        }
    }
}

/// A `Display`able view into a `SourceId` that will write it as a url
pub struct SourceIdToUrl<'a> {
    inner: &'a SourceIdInner,
}

impl<'a> fmt::Display for SourceIdToUrl<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.inner {
            SourceIdInner {
                kind: Kind::Path,
                ref url,
                ..
            } => write!(f, "path+{}", url),
            SourceIdInner {
                kind: Kind::Git(ref reference),
                ref url,
                ref precise,
                ..
            } => {
                write!(f, "git+{}", url)?;
                if let Some(pretty) = reference.pretty_ref() {
                    write!(f, "?{}", pretty)?;
                }
                if let Some(precise) = precise.as_ref() {
                    write!(f, "#{}", precise)?;
                }
                Ok(())
            }
            SourceIdInner {
                kind: Kind::Registry,
                ref url,
                ..
            } => write!(f, "registry+{}", url),
            SourceIdInner {
                kind: Kind::LocalRegistry,
                ref url,
                ..
            } => write!(f, "local-registry+{}", url),
            SourceIdInner {
                kind: Kind::Directory,
                ref url,
                ..
            } => write!(f, "directory+{}", url),
        }
    }
}

impl GitReference {
    /// Returns a `Display`able view of this git reference, or None if using
    /// the head of the "master" branch
    pub fn pretty_ref(&self) -> Option<PrettyRef<'_>> {
        match *self {
            GitReference::Branch(ref s) if *s == "master" => None,
            _ => Some(PrettyRef { inner: self }),
        }
    }
}

/// A git reference that can be `Display`ed
pub struct PrettyRef<'a> {
    inner: &'a GitReference,
}

impl<'a> fmt::Display for PrettyRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.inner {
            GitReference::Branch(ref b) => write!(f, "branch={}", b),
            GitReference::Tag(ref s) => write!(f, "tag={}", s),
            GitReference::Rev(ref s) => write!(f, "rev={}", s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GitReference, Kind, SourceId};
    use crate::util::ToUrl;

    #[test]
    fn github_sources_equal() {
        let loc = "https://github.com/foo/bar".to_url().unwrap();
        let master = Kind::Git(GitReference::Branch("master".to_string()));
        let s1 = SourceId::new(master.clone(), loc).unwrap();

        let loc = "git://github.com/foo/bar".to_url().unwrap();
        let s2 = SourceId::new(master, loc.clone()).unwrap();

        assert_eq!(s1, s2);

        let foo = Kind::Git(GitReference::Branch("foo".to_string()));
        let s3 = SourceId::new(foo, loc).unwrap();
        assert_ne!(s1, s3);
    }
}
