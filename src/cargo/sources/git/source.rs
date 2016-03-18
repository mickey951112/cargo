use std::fmt::{self, Debug, Formatter};
use std::hash::{Hash, Hasher, SipHasher};
use std::mem;

use url::{self, Url};

use core::source::{Source, SourceId};
use core::GitReference;
use core::{Package, PackageId, Summary, Registry, Dependency};
use util::{CargoResult, Config, FileLock, to_hex};
use sources::PathSource;
use sources::git::utils::{GitRemote, GitRevision};

/* TODO: Refactor GitSource to delegate to a PathSource
 */
pub struct GitSource<'cfg> {
    remote: GitRemote,
    reference: GitReference,
    source_id: SourceId,
    path_source: Option<PathSource<'cfg>>,
    rev: Option<GitRevision>,
    checkout_lock: Option<FileLock>,
    ident: String,
    config: &'cfg Config,
}

impl<'cfg> GitSource<'cfg> {
    pub fn new(source_id: &SourceId,
               config: &'cfg Config) -> GitSource<'cfg> {
        assert!(source_id.is_git(), "id is not git, id={}", source_id);

        let remote = GitRemote::new(source_id.url());
        let ident = ident(source_id.url());

        let reference = match source_id.precise() {
            Some(s) => GitReference::Rev(s.to_string()),
            None => source_id.git_reference().unwrap().clone(),
        };

        GitSource {
            remote: remote,
            reference: reference,
            source_id: source_id.clone(),
            path_source: None,
            rev: None,
            checkout_lock: None,
            ident: ident,
            config: config,
        }
    }

    pub fn url(&self) -> &Url { self.remote.url() }

    pub fn read_packages(&mut self) -> CargoResult<Vec<Package>> {
        if self.path_source.is_none() {
            try!(self.update());
        }
        self.path_source.as_mut().unwrap().read_packages()
    }
}

fn ident(url: &Url) -> String {
    let mut hasher = SipHasher::new_with_keys(0,0);

    // FIXME: this really should be able to not use to_str() everywhere, but the
    //        compiler seems to currently ask for static lifetimes spuriously.
    //        Perhaps related to rust-lang/rust#15144
    let url = canonicalize_url(url);
    let ident = url.path().unwrap_or(&[])
                   .last().map(|a| a.clone()).unwrap_or(String::new());

    let ident = if ident == "" {
        "_empty".to_string()
    } else {
        ident
    };

    url.hash(&mut hasher);
    format!("{}-{}", ident, to_hex(hasher.finish()))
}

// Some hacks and heuristics for making equivalent URLs hash the same
pub fn canonicalize_url(url: &Url) -> Url {
    let mut url = url.clone();

    // Strip a trailing slash
    if let url::SchemeData::Relative(ref mut rel) = url.scheme_data {
        if rel.path.last().map(|s| s.is_empty()).unwrap_or(false) {
            rel.path.pop();
        }
    }

    // HACKHACK: For github URL's specifically just lowercase
    // everything.  GitHub treats both the same, but they hash
    // differently, and we're gonna be hashing them. This wants a more
    // general solution, and also we're almost certainly not using the
    // same case conversion rules that GitHub does. (#84)
    if url.domain() == Some("github.com") {
        url.scheme = "https".to_string();
        if let url::SchemeData::Relative(ref mut rel) = url.scheme_data {
            rel.port = Some(443);
            rel.default_port = Some(443);
            let path = mem::replace(&mut rel.path, Vec::new());
            rel.path = path.into_iter().map(|s| {
                s.chars().flat_map(|c| c.to_lowercase()).collect()
            }).collect();
        }
    }

    // Repos generally can be accessed with or w/o '.git'
    if let url::SchemeData::Relative(ref mut rel) = url.scheme_data {
        let needs_chopping = {
            let last = rel.path.last().map(|s| &s[..]).unwrap_or("");
            last.ends_with(".git")
        };
        if needs_chopping {
            let last = rel.path.pop().unwrap();
            rel.path.push(last[..last.len() - 4].to_string())
        }
    }

    url
}

impl<'cfg> Debug for GitSource<'cfg> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        try!(write!(f, "git repo at {}", self.remote.url()));

        match self.reference.to_ref_string() {
            Some(s) => write!(f, " ({})", s),
            None => Ok(())
        }
    }
}

impl<'cfg> Registry for GitSource<'cfg> {
    fn query(&mut self, dep: &Dependency) -> CargoResult<Vec<Summary>> {
        let src = self.path_source.as_mut()
                      .expect("BUG: update() must be called before query()");
        src.query(dep)
    }
}

impl<'cfg> Source for GitSource<'cfg> {
    fn update(&mut self) -> CargoResult<()> {
        // First, lock both the global database and checkout locations that
        // we're going to use. We may be performing a fetch into these locations
        // so we need writable access.
        let db_lock = format!(".cargo-lock-{}", self.ident);
        let db_lock = try!(self.config.git_db_path()
                                      .open_rw(&db_lock, self.config,
                                               "the git database"));
        let db_path = db_lock.parent().join(&self.ident);

        let reference_path = match self.source_id.git_reference() {
            Some(&GitReference::Branch(ref s)) |
            Some(&GitReference::Tag(ref s)) |
            Some(&GitReference::Rev(ref s)) => s,
            None => panic!("not a git source"),
        };
        let checkout_lock = format!(".cargo-lock-{}-{}", self.ident,
                                    reference_path);
        let checkout_lock = try!(self.config.git_checkout_path()
                                     .join(&self.ident)
                                     .open_rw(&checkout_lock, self.config,
                                              "the git checkout"));
        let checkout_path = checkout_lock.parent().join(reference_path);

        // Resolve our reference to an actual revision, and check if the
        // databaes already has that revision. If it does, we just load a
        // database pinned at that revision, and if we don't we issue an update
        // to try to find the revision.
        let actual_rev = self.remote.rev_for(&db_path, &self.reference);
        let should_update = actual_rev.is_err() ||
                            self.source_id.precise().is_none();

        let (repo, actual_rev) = if should_update {
            try!(self.config.shell().status("Updating",
                format!("git repository `{}`", self.remote.url())));

            trace!("updating git source `{:?}`", self.remote);
            let repo = try!(self.remote.checkout(&db_path));
            let rev = try!(repo.rev_for(&self.reference));
            (repo, rev)
        } else {
            (try!(self.remote.db_at(&db_path)), actual_rev.unwrap())
        };

        // Copy the database to the checkout location. After this we could drop
        // the lock on the database as we no longer needed it, but we leave it
        // in scope so the destructors here won't tamper with too much.
        try!(repo.copy_to(actual_rev.clone(), &checkout_path));

        let source_id = self.source_id.with_precise(Some(actual_rev.to_string()));
        let path_source = PathSource::new_recursive(&checkout_path,
                                                    &source_id,
                                                    self.config);

        // Cache the information we just learned, and crucially also cache the
        // lock on the checkout location. We wouldn't want someone else to come
        // swipe our checkout location to another revision while we're using it!
        self.path_source = Some(path_source);
        self.rev = Some(actual_rev);
        self.checkout_lock = Some(checkout_lock);
        self.path_source.as_mut().unwrap().update()
    }

    fn download(&mut self, id: &PackageId) -> CargoResult<Package> {
        trace!("getting packages for package id `{}` from `{:?}`", id,
               self.remote);
        self.path_source.as_mut()
                        .expect("BUG: update() must be called before get()")
                        .download(id)
    }

    fn fingerprint(&self, _pkg: &Package) -> CargoResult<String> {
        Ok(self.rev.as_ref().unwrap().to_string())
    }
}

#[cfg(test)]
mod test {
    use url::Url;
    use super::ident;
    use util::ToUrl;

    #[test]
    pub fn test_url_to_path_ident_with_path() {
        let ident = ident(&url("https://github.com/carlhuda/cargo"));
        assert!(ident.starts_with("cargo-"));
    }

    #[test]
    pub fn test_url_to_path_ident_without_path() {
        let ident = ident(&url("https://github.com"));
        assert!(ident.starts_with("_empty-"));
    }

    #[test]
    fn test_canonicalize_idents_by_stripping_trailing_url_slash() {
        let ident1 = ident(&url("https://github.com/PistonDevelopers/piston/"));
        let ident2 = ident(&url("https://github.com/PistonDevelopers/piston"));
        assert_eq!(ident1, ident2);
    }

    #[test]
    fn test_canonicalize_idents_by_lowercasing_github_urls() {
        let ident1 = ident(&url("https://github.com/PistonDevelopers/piston"));
        let ident2 = ident(&url("https://github.com/pistondevelopers/piston"));
        assert_eq!(ident1, ident2);
    }

    #[test]
    fn test_canonicalize_idents_by_stripping_dot_git() {
        let ident1 = ident(&url("https://github.com/PistonDevelopers/piston"));
        let ident2 = ident(&url("https://github.com/PistonDevelopers/piston.git"));
        assert_eq!(ident1, ident2);
    }

    #[test]
    fn test_canonicalize_idents_different_protocls() {
        let ident1 = ident(&url("https://github.com/PistonDevelopers/piston"));
        let ident2 = ident(&url("git://github.com/PistonDevelopers/piston"));
        assert_eq!(ident1, ident2);
    }

    fn url(s: &str) -> Url {
        s.to_url().unwrap()
    }
}
