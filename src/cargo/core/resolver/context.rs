use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::rc::Rc;

// "ensure" seems to require "bail" be in scope (macro hygiene issue?).
#[allow(unused_imports)]
use failure::{bail, ensure};
use log::debug;

use crate::core::interning::InternedString;
use crate::core::{Dependency, FeatureValue, PackageId, SourceId, Summary};
use crate::util::CargoResult;
use crate::util::Graph;

use super::errors::ActivateResult;
use super::types::{ConflictMap, ConflictReason, DepInfo, Method, RegistryQueryer};

pub use super::encode::{EncodableDependency, EncodablePackageId, EncodableResolve};
pub use super::encode::{Metadata, WorkspaceResolve};
pub use super::resolve::Resolve;

// A `Context` is basically a bunch of local resolution information which is
// kept around for all `BacktrackFrame` instances. As a result, this runs the
// risk of being cloned *a lot* so we want to make this as cheap to clone as
// possible.
#[derive(Clone)]
pub struct Context {
    pub activations: Activations,
    /// list the features that are activated for each package
    pub resolve_features: im_rc::HashMap<PackageId, Rc<HashSet<InternedString>>>,
    /// get the package that will be linking to a native library by its links attribute
    pub links: im_rc::HashMap<InternedString, PackageId>,
    /// for each package the list of names it can see,
    /// then for each name the exact version that name represents and weather the name is public.
    pub public_dependency:
        Option<im_rc::HashMap<PackageId, im_rc::HashMap<InternedString, (PackageId, bool)>>>,

    /// a way to look up for a package in activations what packages required it
    /// and all of the exact deps that it fulfilled.
    pub parents: Graph<PackageId, Rc<Vec<Dependency>>>,
}

/// When backtracking it can be useful to know how far back to go.
/// The `ContextAge` of a `Context` is a monotonically increasing counter of the number
/// of decisions made to get to this state.
/// Several structures store the `ContextAge` when it was added,
/// to be used in `find_candidate` for backtracking.
pub type ContextAge = usize;

/// Find the activated version of a crate based on the name, source, and semver compatibility.
/// By storing this in a hash map we ensure that there is only one
/// semver compatible version of each crate.
/// This all so stores the `ContextAge`.
pub type Activations =
    im_rc::HashMap<(InternedString, SourceId, SemverCompatibility), (Summary, ContextAge)>;

/// A type that represents when cargo treats two Versions as compatible.
/// Versions `a` and `b` are compatible if their left-most nonzero digit is the
/// same.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum SemverCompatibility {
    Major(NonZeroU64),
    Minor(NonZeroU64),
    Patch(u64),
}

impl From<&semver::Version> for SemverCompatibility {
    fn from(ver: &semver::Version) -> Self {
        if let Some(m) = NonZeroU64::new(ver.major) {
            return SemverCompatibility::Major(m);
        }
        if let Some(m) = NonZeroU64::new(ver.minor) {
            return SemverCompatibility::Minor(m);
        }
        SemverCompatibility::Patch(ver.patch)
    }
}

impl PackageId {
    pub fn as_activations_key(self) -> (InternedString, SourceId, SemverCompatibility) {
        (self.name(), self.source_id(), self.version().into())
    }
}

impl Context {
    pub fn new(check_public_visible_dependencies: bool) -> Context {
        Context {
            resolve_features: im_rc::HashMap::new(),
            links: im_rc::HashMap::new(),
            public_dependency: if check_public_visible_dependencies {
                Some(im_rc::HashMap::new())
            } else {
                None
            },
            parents: Graph::new(),
            activations: im_rc::HashMap::new(),
        }
    }

    /// Activate this summary by inserting it into our list of known activations.
    ///
    /// Returns `true` if this summary with the given method is already activated.
    pub fn flag_activated(&mut self, summary: &Summary, method: &Method<'_>) -> CargoResult<bool> {
        let id = summary.package_id();
        let age: ContextAge = self.age();
        match self.activations.entry(id.as_activations_key()) {
            im_rc::hashmap::Entry::Occupied(o) => {
                debug_assert_eq!(
                    &o.get().0,
                    summary,
                    "cargo does not allow two semver compatible versions"
                );
            }
            im_rc::hashmap::Entry::Vacant(v) => {
                if let Some(link) = summary.links() {
                    ensure!(
                        self.links.insert(link, id).is_none(),
                        "Attempting to resolve a dependency with more then one crate with the \
                         links={}.\nThis will not build as is. Consider rebuilding the .lock file.",
                        &*link
                    );
                }
                v.insert((summary.clone(), age));
                return Ok(false);
            }
        }
        debug!("checking if {} is already activated", summary.package_id());
        let (features, use_default) = match *method {
            Method::Everything
            | Method::Required {
                all_features: true, ..
            } => return Ok(false),
            Method::Required {
                features,
                uses_default_features,
                ..
            } => (features, uses_default_features),
        };

        let has_default_feature = summary.features().contains_key("default");
        Ok(match self.resolve_features.get(&id) {
            Some(prev) => {
                features.iter().all(|f| prev.contains(f))
                    && (!use_default || prev.contains("default") || !has_default_feature)
            }
            None => features.is_empty() && (!use_default || !has_default_feature),
        })
    }

    pub fn build_deps(
        &mut self,
        registry: &mut RegistryQueryer<'_>,
        parent: Option<&Summary>,
        candidate: &Summary,
        method: &Method<'_>,
    ) -> ActivateResult<Vec<DepInfo>> {
        // First, figure out our set of dependencies based on the requested set
        // of features. This also calculates what features we're going to enable
        // for our own dependencies.
        let deps = self.resolve_features(parent, candidate, method)?;

        // Next, transform all dependencies into a list of possible candidates
        // which can satisfy that dependency.
        let mut deps = deps
            .into_iter()
            .map(|(dep, features)| {
                let candidates = registry.query(&dep)?;
                Ok((dep, candidates, Rc::new(features)))
            })
            .collect::<CargoResult<Vec<DepInfo>>>()?;

        // Attempt to resolve dependencies with fewer candidates before trying
        // dependencies with more candidates. This way if the dependency with
        // only one candidate can't be resolved we don't have to do a bunch of
        // work before we figure that out.
        deps.sort_by_key(|&(_, ref a, _)| a.len());

        Ok(deps)
    }

    /// Returns the `ContextAge` of this `Context`.
    /// For now we use (len of activations) as the age.
    /// See the `ContextAge` docs for more details.
    pub fn age(&self) -> ContextAge {
        self.activations.len()
    }

    /// If the package is active returns the `ContextAge` when it was added
    pub fn is_active(&self, id: PackageId) -> Option<ContextAge> {
        self.activations
            .get(&id.as_activations_key())
            .and_then(|(s, l)| if s.package_id() == id { Some(*l) } else { None })
    }

    /// Checks whether all of `parent` and the keys of `conflicting activations`
    /// are still active.
    /// If so returns the `ContextAge` when the newest one was added.
    pub fn is_conflicting(
        &self,
        parent: Option<PackageId>,
        conflicting_activations: &ConflictMap,
    ) -> Option<usize> {
        let mut max = 0;
        for &id in conflicting_activations.keys().chain(parent.as_ref()) {
            if let Some(age) = self.is_active(id) {
                max = std::cmp::max(max, age);
            } else {
                return None;
            }
        }
        Some(max)
    }

    /// Returns all dependencies and the features we want from them.
    pub fn resolve_features<'b>(
        &mut self,
        parent: Option<&Summary>,
        s: &'b Summary,
        method: &'b Method<'_>,
    ) -> ActivateResult<Vec<(Dependency, Vec<InternedString>)>> {
        let dev_deps = match *method {
            Method::Everything => true,
            Method::Required { dev_deps, .. } => dev_deps,
        };

        // First, filter by dev-dependencies.
        let deps = s.dependencies();
        let deps = deps.iter().filter(|d| d.is_transitive() || dev_deps);

        let reqs = build_requirements(s, method)?;
        let mut ret = Vec::new();
        let mut used_features = HashSet::new();
        let default_dep = (false, Vec::new());

        // Next, collect all actually enabled dependencies and their features.
        for dep in deps {
            // Skip optional dependencies, but not those enabled through a
            // feature
            if dep.is_optional() && !reqs.deps.contains_key(&dep.name_in_toml()) {
                continue;
            }
            // So we want this dependency. Move the features we want from
            // `feature_deps` to `ret` and register ourselves as using this
            // name.
            let base = reqs.deps.get(&dep.name_in_toml()).unwrap_or(&default_dep);
            used_features.insert(dep.name_in_toml());
            let always_required = !dep.is_optional()
                && !s
                    .dependencies()
                    .iter()
                    .any(|d| d.is_optional() && d.name_in_toml() == dep.name_in_toml());
            if always_required && base.0 {
                return Err(match parent {
                    None => failure::format_err!(
                        "Package `{}` does not have feature `{}`. It has a required dependency \
                         with that name, but only optional dependencies can be used as features.",
                        s.package_id(),
                        dep.name_in_toml()
                    )
                    .into(),
                    Some(p) => (
                        p.package_id(),
                        ConflictReason::RequiredDependencyAsFeatures(dep.name_in_toml()),
                    )
                        .into(),
                });
            }
            let mut base = base.1.clone();
            base.extend(dep.features().iter());
            for feature in base.iter() {
                if feature.contains('/') {
                    return Err(failure::format_err!(
                        "feature names may not contain slashes: `{}`",
                        feature
                    )
                    .into());
                }
            }
            ret.push((dep.clone(), base));
        }

        // Any entries in `reqs.dep` which weren't used are bugs in that the
        // package does not actually have those dependencies. We classified
        // them as dependencies in the first place because there is no such
        // feature, either.
        let remaining = reqs
            .deps
            .keys()
            .cloned()
            .filter(|s| !used_features.contains(s))
            .collect::<Vec<_>>();
        if !remaining.is_empty() {
            let features = remaining.join(", ");
            return Err(match parent {
                None => failure::format_err!(
                    "Package `{}` does not have these features: `{}`",
                    s.package_id(),
                    features
                )
                .into(),
                Some(p) => (p.package_id(), ConflictReason::MissingFeatures(features)).into(),
            });
        }

        // Record what list of features is active for this package.
        if !reqs.used.is_empty() {
            Rc::make_mut(
                self.resolve_features
                    .entry(s.package_id())
                    .or_insert_with(|| Rc::new(HashSet::with_capacity(reqs.used.len()))),
            )
            .extend(reqs.used);
        }

        Ok(ret)
    }

    pub fn resolve_replacements(
        &self,
        registry: &RegistryQueryer<'_>,
    ) -> HashMap<PackageId, PackageId> {
        self.activations
            .values()
            .filter_map(|(s, _)| registry.used_replacement_for(s.package_id()))
            .collect()
    }

    pub fn graph(&self) -> Graph<PackageId, Vec<Dependency>> {
        let mut graph: Graph<PackageId, Vec<Dependency>> = Graph::new();
        self.activations
            .values()
            .for_each(|(r, _)| graph.add(r.package_id()));
        for i in self.parents.iter() {
            graph.add(*i);
            for (o, e) in self.parents.edges(i) {
                let old_link = graph.link(*o, *i);
                assert!(old_link.is_empty());
                *old_link = e.to_vec();
            }
        }
        graph
    }
}

/// Takes requested features for a single package from the input `Method` and
/// recurses to find all requested features, dependencies and requested
/// dependency features in a `Requirements` object, returning it to the resolver.
fn build_requirements<'a, 'b: 'a>(
    s: &'a Summary,
    method: &'b Method<'_>,
) -> CargoResult<Requirements<'a>> {
    let mut reqs = Requirements::new(s);

    match *method {
        Method::Everything
        | Method::Required {
            all_features: true, ..
        } => {
            for key in s.features().keys() {
                reqs.require_feature(*key)?;
            }
            for dep in s.dependencies().iter().filter(|d| d.is_optional()) {
                reqs.require_dependency(dep.name_in_toml());
            }
        }
        Method::Required {
            all_features: false,
            features: requested,
            ..
        } => {
            for &f in requested.iter() {
                reqs.require_value(&FeatureValue::new(f, s))?;
            }
        }
    }
    match *method {
        Method::Everything
        | Method::Required {
            uses_default_features: true,
            ..
        } => {
            if s.features().contains_key("default") {
                reqs.require_feature(InternedString::new("default"))?;
            }
        }
        Method::Required {
            uses_default_features: false,
            ..
        } => {}
    }
    Ok(reqs)
}

struct Requirements<'a> {
    summary: &'a Summary,
    // The deps map is a mapping of package name to list of features enabled.
    // Each package should be enabled, and each package should have the
    // specified set of features enabled. The boolean indicates whether this
    // package was specifically requested (rather than just requesting features
    // *within* this package).
    deps: HashMap<InternedString, (bool, Vec<InternedString>)>,
    // The used features set is the set of features which this local package had
    // enabled, which is later used when compiling to instruct the code what
    // features were enabled.
    used: HashSet<InternedString>,
    visited: HashSet<InternedString>,
}

impl Requirements<'_> {
    fn new(summary: &Summary) -> Requirements<'_> {
        Requirements {
            summary,
            deps: HashMap::new(),
            used: HashSet::new(),
            visited: HashSet::new(),
        }
    }

    fn require_crate_feature(&mut self, package: InternedString, feat: InternedString) {
        self.used.insert(package);
        self.deps
            .entry(package)
            .or_insert((false, Vec::new()))
            .1
            .push(feat);
    }

    fn seen(&mut self, feat: InternedString) -> bool {
        if self.visited.insert(feat) {
            self.used.insert(feat);
            false
        } else {
            true
        }
    }

    fn require_dependency(&mut self, pkg: InternedString) {
        if self.seen(pkg) {
            return;
        }
        self.deps.entry(pkg).or_insert((false, Vec::new())).0 = true;
    }

    fn require_feature(&mut self, feat: InternedString) -> CargoResult<()> {
        if feat.is_empty() || self.seen(feat) {
            return Ok(());
        }
        for fv in self
            .summary
            .features()
            .get(feat.as_str())
            .expect("must be a valid feature")
        {
            match *fv {
                FeatureValue::Feature(ref dep_feat) if **dep_feat == *feat => failure::bail!(
                    "cyclic feature dependency: feature `{}` depends on itself",
                    feat
                ),
                _ => {}
            }
            self.require_value(fv)?;
        }
        Ok(())
    }

    fn require_value(&mut self, fv: &FeatureValue) -> CargoResult<()> {
        match fv {
            FeatureValue::Feature(feat) => self.require_feature(*feat)?,
            FeatureValue::Crate(dep) => self.require_dependency(*dep),
            FeatureValue::CrateFeature(dep, dep_feat) => {
                self.require_crate_feature(*dep, *dep_feat)
            }
        };
        Ok(())
    }
}
