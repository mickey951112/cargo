use std::collections::HashMap;
use std::mem;

use semver::Version;
use core::{Dependency, PackageId, SourceId};

use util::CargoResult;

/// Subset of a `Manifest`. Contains only the most important informations about
/// a package.
///
/// Summaries are cloned, and should not be mutated after creation
#[derive(Debug,Clone)]
pub struct Summary {
    package_id: PackageId,
    dependencies: Vec<Dependency>,
    features: HashMap<String, Vec<String>>,
}

impl Summary {
    pub fn new(pkg_id: PackageId,
               dependencies: Vec<Dependency>,
               features: HashMap<String, Vec<String>>) -> CargoResult<Summary> {
        for dep in dependencies.iter() {
            if features.get(dep.name()).is_some() {
                bail!("Features and dependencies cannot have the \
                       same name: `{}`", dep.name())
            }
            if dep.is_optional() && !dep.is_transitive() {
                bail!("Dev-dependencies are not allowed to be optional: `{}`",
                      dep.name())
            }
        }
        for (feature, list) in features.iter() {
            for dep in list.iter() {
                let mut parts = dep.splitn(2, '/');
                let dep = parts.next().unwrap();
                let is_reexport = parts.next().is_some();
                if !is_reexport && features.get(dep).is_some() { continue }
                match dependencies.iter().find(|d| d.name() == dep) {
                    Some(d) => {
                        if d.is_optional() || is_reexport { continue }
                        bail!("Feature `{}` depends on `{}` which is not an \
                               optional dependency.\nConsider adding \
                               `optional = true` to the dependency",
                               feature, dep)
                    }
                    None if is_reexport => {
                        bail!("Feature `{}` requires `{}` which is not an \
                               optional dependency", feature, dep)
                    }
                    None => {
                        bail!("Feature `{}` includes `{}` which is neither \
                               a dependency nor another feature", feature, dep)
                    }
                }
            }
        }
        Ok(Summary {
            package_id: pkg_id,
            dependencies: dependencies,
            features: features,
        })
    }

    pub fn package_id(&self) -> &PackageId { &self.package_id }
    pub fn name(&self) -> &str { self.package_id().name() }
    pub fn version(&self) -> &Version { self.package_id().version() }
    pub fn source_id(&self) -> &SourceId { self.package_id.source_id() }
    pub fn dependencies(&self) -> &[Dependency] { &self.dependencies }
    pub fn features(&self) -> &HashMap<String, Vec<String>> { &self.features }

    pub fn override_id(mut self, id: PackageId) -> Summary {
        self.package_id = id;
        self
    }

    pub fn map_dependencies<F>(mut self, f: F) -> Summary
                               where F: FnMut(Dependency) -> Dependency {
        let deps = mem::replace(&mut self.dependencies, Vec::new());
        self.dependencies = deps.into_iter().map(f).collect();
        self
    }
}

impl PartialEq for Summary {
    fn eq(&self, other: &Summary) -> bool {
        self.package_id == other.package_id
    }
}

pub trait SummaryVec {
    fn names(&self) -> Vec<String>;
}

impl SummaryVec for Vec<Summary> {
    // TODO: Move to Registry
    fn names(&self) -> Vec<String> {
        self.iter().map(|summary| summary.name().to_string()).collect()
    }

}
