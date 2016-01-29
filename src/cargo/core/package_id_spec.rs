use std::collections::HashMap;
use std::fmt;

use semver::Version;
use url::{self, Url, UrlParser};

use core::PackageId;
use util::{CargoResult, ToUrl, human, ToSemver, ChainError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PackageIdSpec {
    name: String,
    version: Option<Version>,
    url: Option<Url>,
}

impl PackageIdSpec {
    pub fn parse(spec: &str) -> CargoResult<PackageIdSpec> {
        if spec.contains("/") {
            match spec.to_url() {
                Ok(url) => return PackageIdSpec::from_url(url),
                Err(..) => {}
            }
            if !spec.contains("://") {
                match url(&format!("cargo://{}", spec)) {
                    Ok(url) => return PackageIdSpec::from_url(url),
                    Err(..) => {}
                }
            }
        }
        let mut parts = spec.splitn(2, ':');
        let name = parts.next().unwrap();
        let version = match parts.next() {
            Some(version) => Some(try!(Version::parse(version).map_err(human))),
            None => None,
        };
        for ch in name.chars() {
            if !ch.is_alphanumeric() && ch != '_' && ch != '-' {
                bail!("invalid character in pkgid `{}`: `{}`", spec, ch)
            }
        }
        Ok(PackageIdSpec {
            name: name.to_string(),
            version: version,
            url: None,
        })
    }

    pub fn query_str<'a, I>(spec: &str, i: I) -> CargoResult<&'a PackageId>
        where I: IntoIterator<Item=&'a PackageId>
    {
        let spec = try!(PackageIdSpec::parse(spec).chain_error(|| {
            human(format!("invalid package id specification: `{}`", spec))
        }));
        spec.query(i)
    }

    pub fn from_package_id(package_id: &PackageId) -> PackageIdSpec {
        PackageIdSpec {
            name: package_id.name().to_string(),
            version: Some(package_id.version().clone()),
            url: Some(package_id.source_id().url().clone()),
        }
    }

    fn from_url(mut url: Url) -> CargoResult<PackageIdSpec> {
        if url.query.is_some() {
            bail!("cannot have a query string in a pkgid: {}", url)
        }
        let frag = url.fragment.take();
        let (name, version) = {
            let path = try!(url.path().chain_error(|| {
                human(format!("pkgid urls must have a path: {}", url))
            }));
            let path_name = try!(path.last().chain_error(|| {
                human(format!("pkgid urls must have at least one path \
                               component: {}", url))
            }));
            match frag {
                Some(fragment) => {
                    let mut parts = fragment.splitn(2, ':');
                    let name_or_version = parts.next().unwrap();
                    match parts.next() {
                        Some(part) => {
                            let version = try!(part.to_semver().map_err(human));
                            (name_or_version.to_string(), Some(version))
                        }
                        None => {
                            if name_or_version.chars().next().unwrap()
                                              .is_alphabetic() {
                                (name_or_version.to_string(), None)
                            } else {
                                let version = try!(name_or_version.to_semver()
                                                                  .map_err(human));
                                (path_name.to_string(), Some(version))
                            }
                        }
                    }
                }
                None => (path_name.to_string(), None),
            }
        };
        Ok(PackageIdSpec {
            name: name,
            version: version,
            url: Some(url),
        })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn version(&self) -> Option<&Version> { self.version.as_ref() }
    pub fn url(&self) -> Option<&Url> { self.url.as_ref() }

    pub fn matches(&self, package_id: &PackageId) -> bool {
        if self.name() != package_id.name() { return false }

        match self.version {
            Some(ref v) => if v != package_id.version() { return false },
            None => {}
        }

        match self.url {
            Some(ref u) => u == package_id.source_id().url(),
            None => true
        }
    }

    pub fn query<'a, I>(&self, i: I) -> CargoResult<&'a PackageId>
        where I: IntoIterator<Item=&'a PackageId>
    {
        let mut ids = i.into_iter().filter(|p| self.matches(*p));
        let ret = match ids.next() {
            Some(id) => id,
            None => bail!("package id specification `{}` \
                           matched no packages", self),
        };
        return match ids.next() {
            Some(other) => {
                let mut msg = format!("There are multiple `{}` packages in \
                                       your project, and the specification \
                                       `{}` is ambiguous.\n\
                                       Please re-run this command \
                                       with `-p <spec>` where `<spec>` is one \
                                       of the following:",
                                      self.name(), self);
                let mut vec = vec![ret, other];
                vec.extend(ids);
                minimize(&mut msg, vec, self);
                Err(human(msg))
            }
            None => Ok(ret)
        };

        fn minimize(msg: &mut String,
                    ids: Vec<&PackageId>,
                    spec: &PackageIdSpec) {
            let mut version_cnt = HashMap::new();
            for id in ids.iter() {
                *version_cnt.entry(id.version()).or_insert(0) += 1;
            }
            for id in ids.iter() {
                if version_cnt[id.version()] == 1 {
                    msg.push_str(&format!("\n  {}:{}", spec.name(),
                                          id.version()));
                } else {
                    msg.push_str(&format!("\n  {}",
                                          PackageIdSpec::from_package_id(*id)));
                }
            }
        }
    }
}

fn url(s: &str) -> url::ParseResult<Url> {
    return UrlParser::new().scheme_type_mapper(mapper).parse(s);

    fn mapper(scheme: &str) -> url::SchemeType {
        if scheme == "cargo" {
            url::SchemeType::Relative(1)
        } else {
            url::whatwg_scheme_type_mapper(scheme)
        }
    }

}

impl fmt::Display for PackageIdSpec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut printed_name = false;
        match self.url {
            Some(ref url) => {
                if url.scheme == "cargo" {
                    try!(write!(f, "{}/{}", url.host().unwrap(),
                                url.path().unwrap().join("/")));
                } else {
                    try!(write!(f, "{}", url));
                }
                if url.path().unwrap().last().unwrap() != &self.name {
                    printed_name = true;
                    try!(write!(f, "#{}", self.name));
                }
            }
            None => { printed_name = true; try!(write!(f, "{}", self.name)) }
        }
        match self.version {
            Some(ref v) => {
                try!(write!(f, "{}{}", if printed_name {":"} else {"#"}, v));
            }
            None => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::{PackageId, SourceId};
    use super::{PackageIdSpec, url};
    use url::Url;
    use semver::Version;

    #[test]
    fn good_parsing() {
        fn ok(spec: &str, expected: PackageIdSpec) {
            let parsed = PackageIdSpec::parse(spec).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), spec);
        }

        ok("http://crates.io/foo#1.2.3", PackageIdSpec {
            name: "foo".to_string(),
            version: Some(Version::parse("1.2.3").unwrap()),
            url: Some(url("http://crates.io/foo").unwrap()),
        });
        ok("http://crates.io/foo#bar:1.2.3", PackageIdSpec {
            name: "bar".to_string(),
            version: Some(Version::parse("1.2.3").unwrap()),
            url: Some(url("http://crates.io/foo").unwrap()),
        });
        ok("crates.io/foo", PackageIdSpec {
            name: "foo".to_string(),
            version: None,
            url: Some(url("cargo://crates.io/foo").unwrap()),
        });
        ok("crates.io/foo#1.2.3", PackageIdSpec {
            name: "foo".to_string(),
            version: Some(Version::parse("1.2.3").unwrap()),
            url: Some(url("cargo://crates.io/foo").unwrap()),
        });
        ok("crates.io/foo#bar", PackageIdSpec {
            name: "bar".to_string(),
            version: None,
            url: Some(url("cargo://crates.io/foo").unwrap()),
        });
        ok("crates.io/foo#bar:1.2.3", PackageIdSpec {
            name: "bar".to_string(),
            version: Some(Version::parse("1.2.3").unwrap()),
            url: Some(url("cargo://crates.io/foo").unwrap()),
        });
        ok("foo", PackageIdSpec {
            name: "foo".to_string(),
            version: None,
            url: None,
        });
        ok("foo:1.2.3", PackageIdSpec {
            name: "foo".to_string(),
            version: Some(Version::parse("1.2.3").unwrap()),
            url: None,
        });
    }

    #[test]
    fn bad_parsing() {
        assert!(PackageIdSpec::parse("baz:").is_err());
        assert!(PackageIdSpec::parse("baz:1.0").is_err());
        assert!(PackageIdSpec::parse("http://baz:1.0").is_err());
        assert!(PackageIdSpec::parse("http://#baz:1.0").is_err());
    }

    #[test]
    fn matching() {
        let url = Url::parse("http://example.com").unwrap();
        let sid = SourceId::for_registry(&url);
        let foo = PackageId::new("foo", "1.2.3", &sid).unwrap();
        let bar = PackageId::new("bar", "1.2.3", &sid).unwrap();

        assert!( PackageIdSpec::parse("foo").unwrap().matches(&foo));
        assert!(!PackageIdSpec::parse("foo").unwrap().matches(&bar));
        assert!( PackageIdSpec::parse("foo:1.2.3").unwrap().matches(&foo));
        assert!(!PackageIdSpec::parse("foo:1.2.2").unwrap().matches(&foo));
    }
}
