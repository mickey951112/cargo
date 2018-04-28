use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use core::interning::InternedString;
use core::{Dependency, FeatureValue, PackageId, SourceId, Summary};
use util::CargoResult;
use util::Graph;

use super::types::RegistryQueryer;
use super::types::{ActivateResult, ConflictReason, DepInfo, GraphNode, Method, RcList};

pub use super::encode::{EncodableDependency, EncodablePackageId, EncodableResolve};
pub use super::encode::{Metadata, WorkspaceResolve};
pub use super::resolve::Resolve;

// A `Context` is basically a bunch of local resolution information which is
// kept around for all `BacktrackFrame` instances. As a result, this runs the
// risk of being cloned *a lot* so we want to make this as cheap to clone as
// possible.
#[derive(Clone)]
pub struct Context {
    // TODO: Both this and the two maps below are super expensive to clone. We should
    //       switch to persistent hash maps if we can at some point or otherwise
    //       make these much cheaper to clone in general.
    pub activations: Activations,
    pub resolve_features: HashMap<PackageId, Rc<HashSet<InternedString>>>,
    pub links: HashMap<InternedString, PackageId>,

    // These are two cheaply-cloneable lists (O(1) clone) which are effectively
    // hash maps but are built up as "construction lists". We'll iterate these
    // at the very end and actually construct the map that we're making.
    pub resolve_graph: RcList<GraphNode>,
    pub resolve_replacements: RcList<(PackageId, PackageId)>,

    // These warnings are printed after resolution.
    pub warnings: RcList<String>,
}

pub type Activations = HashMap<(InternedString, SourceId), Rc<Vec<Summary>>>;

impl Context {
    pub fn new() -> Context {
        Context {
            resolve_graph: RcList::new(),
            resolve_features: HashMap::new(),
            links: HashMap::new(),
            resolve_replacements: RcList::new(),
            activations: HashMap::new(),
            warnings: RcList::new(),
        }
    }

    /// Activate this summary by inserting it into our list of known activations.
    ///
    /// Returns true if this summary with the given method is already activated.
    pub fn flag_activated(&mut self, summary: &Summary, method: &Method) -> CargoResult<bool> {
        let id = summary.package_id();
        let prev = self.activations
            .entry((id.name(), id.source_id().clone()))
            .or_insert_with(|| Rc::new(Vec::new()));
        if !prev.iter().any(|c| c == summary) {
            self.resolve_graph.push(GraphNode::Add(id.clone()));
            if let Some(link) = summary.links() {
                ensure!(
                    self.links.insert(link, id.clone()).is_none(),
                    "Attempting to resolve a with more then one crate with the links={}. \n\
                     This will not build as is. Consider rebuilding the .lock file.",
                    &*link
                );
            }
            Rc::make_mut(prev).push(summary.clone());
            return Ok(false);
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
        Ok(match self.resolve_features.get(id) {
            Some(prev) => {
                features
                    .iter()
                    .all(|f| prev.contains(&InternedString::new(f)))
                    && (!use_default || prev.contains(&InternedString::new("default"))
                        || !has_default_feature)
            }
            None => features.is_empty() && (!use_default || !has_default_feature),
        })
    }

    pub fn build_deps(
        &mut self,
        registry: &mut RegistryQueryer,
        parent: Option<&Summary>,
        candidate: &Summary,
        method: &Method,
    ) -> ActivateResult<Vec<DepInfo>> {
        // First, figure out our set of dependencies based on the requested set
        // of features. This also calculates what features we're going to enable
        // for our own dependencies.
        let deps = self.resolve_features(parent, candidate, method)?;

        // Next, transform all dependencies into a list of possible candidates
        // which can satisfy that dependency.
        let mut deps = deps.into_iter()
            .map(|(dep, features)| {
                let candidates = registry.query(&dep)?;
                Ok((dep, candidates, Rc::new(features)))
            })
            .collect::<CargoResult<Vec<DepInfo>>>()?;

        // Attempt to resolve dependencies with fewer candidates before trying
        // dependencies with more candidates.  This way if the dependency with
        // only one candidate can't be resolved we don't have to do a bunch of
        // work before we figure that out.
        deps.sort_by_key(|&(_, ref a, _)| a.len());

        Ok(deps)
    }

    pub fn prev_active(&self, dep: &Dependency) -> &[Summary] {
        self.activations
            .get(&(dep.name(), dep.source_id().clone()))
            .map(|v| &v[..])
            .unwrap_or(&[])
    }

    fn is_active(&self, id: &PackageId) -> bool {
        self.activations
            .get(&(id.name(), id.source_id().clone()))
            .map(|v| v.iter().any(|s| s.package_id() == id))
            .unwrap_or(false)
    }

    /// checks whether all of `parent` and the keys of `conflicting activations`
    /// are still active
    pub fn is_conflicting(
        &self,
        parent: Option<&PackageId>,
        conflicting_activations: &HashMap<PackageId, ConflictReason>,
    ) -> bool {
        conflicting_activations
            .keys()
            .chain(parent)
            .all(|id| self.is_active(id))
    }

    /// Return all dependencies and the features we want from them.
    fn resolve_features<'b>(
        &mut self,
        parent: Option<&Summary>,
        s: &'b Summary,
        method: &'b Method,
    ) -> ActivateResult<Vec<(Dependency, Vec<InternedString>)>> {
        let dev_deps = match *method {
            Method::Everything => true,
            Method::Required { dev_deps, .. } => dev_deps,
        };

        // First, filter by dev-dependencies
        let deps = s.dependencies();
        let deps = deps.iter().filter(|d| d.is_transitive() || dev_deps);

        // Requested features stored in the Method are stored as string references, but we want to
        // transform them into FeatureValues here. In order to pass the borrow checker with
        // storage of the FeatureValues that outlives the Requirements object, we do the
        // transformation here, and pass the FeatureValues to build_requirements().
        let values = if let Method::Required {
            all_features: false,
            features: requested,
            ..
        } = *method
        {
            requested
                .iter()
                .map(|&f| FeatureValue::new(f, s))
                .collect::<Vec<FeatureValue>>()
        } else {
            vec![]
        };
        let mut reqs = build_requirements(s, method, &values)?;
        let mut ret = Vec::new();

        // Next, collect all actually enabled dependencies and their features.
        for dep in deps {
            // Skip optional dependencies, but not those enabled through a feature
            if dep.is_optional() && !reqs.deps.contains_key(&*dep.name()) {
                continue;
            }
            // So we want this dependency.  Move the features we want from `feature_deps`
            // to `ret`.
            let base = reqs.deps.remove(&*dep.name()).unwrap_or((false, vec![]));
            if !dep.is_optional() && base.0 {
                self.warnings.push(format!(
                    "Package `{}` does not have feature `{}`. It has a required dependency \
                     with that name, but only optional dependencies can be used as features. \
                     This is currently a warning to ease the transition, but it will become an \
                     error in the future.",
                    s.package_id(),
                    dep.name()
                ));
            }
            let mut base = base.1;
            base.extend(dep.features().iter());
            for feature in base.iter() {
                if feature.contains('/') {
                    return Err(
                        format_err!("feature names may not contain slashes: `{}`", feature).into(),
                    );
                }
            }
            ret.push((dep.clone(), base));
        }

        // Any remaining entries in feature_deps are bugs in that the package does not actually
        // have those dependencies.  We classified them as dependencies in the first place
        // because there is no such feature, either.
        if !reqs.deps.is_empty() {
            let unknown = reqs.deps.keys().map(|s| &s[..]).collect::<Vec<&str>>();
            let features = unknown.join(", ");
            return Err(match parent {
                None => format_err!(
                    "Package `{}` does not have these features: `{}`",
                    s.package_id(),
                    features
                ).into(),
                Some(p) => (
                    p.package_id().clone(),
                    ConflictReason::MissingFeatures(features),
                ).into(),
            });
        }

        // Record what list of features is active for this package.
        if !reqs.used.is_empty() {
            let pkgid = s.package_id();

            let set = Rc::make_mut(
                self.resolve_features
                    .entry(pkgid.clone())
                    .or_insert_with(|| Rc::new(HashSet::new())),
            );

            for feature in reqs.used {
                set.insert(InternedString::new(feature));
            }
        }

        Ok(ret)
    }

    pub fn resolve_replacements(&self) -> HashMap<PackageId, PackageId> {
        let mut replacements = HashMap::new();
        let mut cur = &self.resolve_replacements;
        while let Some(ref node) = cur.head {
            let (k, v) = node.0.clone();
            replacements.insert(k, v);
            cur = &node.1;
        }
        replacements
    }

    pub fn graph(&self) -> Graph<PackageId, Vec<Dependency>> {
        let mut graph: Graph<PackageId, Vec<Dependency>> = Graph::new();
        let mut cur = &self.resolve_graph;
        while let Some(ref node) = cur.head {
            match node.0 {
                GraphNode::Add(ref p) => graph.add(p.clone()),
                GraphNode::Link(ref a, ref b, ref dep) => {
                    graph.link(a.clone(), b.clone()).push(dep.clone());
                }
            }
            cur = &node.1;
        }
        graph
    }
}

/// Takes requested features for a single package from the input Method and
/// recurses to find all requested features, dependencies and requested
/// dependency features in a Requirements object, returning it to the resolver.
fn build_requirements<'a, 'b: 'a>(
    s: &'a Summary,
    method: &'b Method,
    requested: &'a [FeatureValue],
) -> CargoResult<Requirements<'a>> {
    let mut reqs = Requirements::new(s);
    for fv in requested.iter() {
        reqs.require_value(fv)?;
    }
    match *method {
        Method::Everything
        | Method::Required {
            all_features: true, ..
        } => {
            for key in s.features().keys() {
                reqs.require_feature(key)?;
            }
            for dep in s.dependencies().iter().filter(|d| d.is_optional()) {
                reqs.require_dependency(dep.name().as_str());
            }
        }
        _ => {} // Explicitly requested features are handled through `requested`
    }
    match *method {
        Method::Everything
        | Method::Required {
            uses_default_features: true,
            ..
        } => {
            if s.features().get("default").is_some() {
                reqs.require_feature("default")?;
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
    deps: HashMap<&'a str, (bool, Vec<InternedString>)>,
    // The used features set is the set of features which this local package had
    // enabled, which is later used when compiling to instruct the code what
    // features were enabled.
    used: HashSet<&'a str>,
    visited: HashSet<&'a str>,
}

impl<'r> Requirements<'r> {
    fn new(summary: &Summary) -> Requirements {
        Requirements {
            summary,
            deps: HashMap::new(),
            used: HashSet::new(),
            visited: HashSet::new(),
        }
    }

    fn require_crate_feature(&mut self, package: &'r str, feat: InternedString) {
        self.used.insert(package);
        self.deps
            .entry(package)
            .or_insert((false, Vec::new()))
            .1
            .push(feat);
    }

    fn seen(&mut self, feat: &'r str) -> bool {
        if self.visited.insert(feat) {
            self.used.insert(feat);
            false
        } else {
            true
        }
    }

    fn require_dependency(&mut self, pkg: &'r str) {
        if self.seen(pkg) {
            return;
        }
        self.deps.entry(pkg).or_insert((false, Vec::new())).0 = true;
    }

    fn require_feature(&mut self, feat: &'r str) -> CargoResult<()> {
        if feat.is_empty() || self.seen(feat) {
            return Ok(());
        }
        for fv in self.summary
            .features()
            .get(feat)
            .expect("must be a valid feature")
        {
            match *fv {
                FeatureValue::Feature(ref dep_feat) if **dep_feat == *feat => bail!(
                    "Cyclic feature dependency: feature `{}` depends on itself",
                    feat
                ),
                _ => {}
            }
            self.require_value(fv)?;
        }
        Ok(())
    }

    fn require_value(&mut self, fv: &'r FeatureValue) -> CargoResult<()> {
        match *fv {
            FeatureValue::Feature(ref feat) => self.require_feature(feat),
            FeatureValue::Crate(ref dep) => Ok(self.require_dependency(dep)),
            FeatureValue::CrateFeature(ref dep, dep_feat) => {
                Ok(self.require_crate_feature(dep, dep_feat))
            }
        }
    }
}
