//! Resolution of the entire dependency graph for a crate.
//!
//! This module implements the core logic in taking the world of crates and
//! constraints and creating a resolved graph with locked versions for all
//! crates and their dependencies. This is separate from the registry module
//! which is more worried about discovering crates from various sources, this
//! module just uses the Registry trait as a source to learn about crates from.
//!
//! Actually solving a constraint graph is an NP-hard problem. This algorithm
//! is basically a nice heuristic to make sure we get roughly the best answer
//! most of the time. The constraints that we're working with are:
//!
//! 1. Each crate can have any number of dependencies. Each dependency can
//!    declare a version range that it is compatible with.
//! 2. Crates can be activated with multiple version (e.g., show up in the
//!    dependency graph twice) so long as each pairwise instance have
//!    semver-incompatible versions.
//!
//! The algorithm employed here is fairly simple, we simply do a DFS, activating
//! the "newest crate" (highest version) first and then going to the next
//! option. The heuristics we employ are:
//!
//! * Never try to activate a crate version which is incompatible. This means we
//!   only try crates which will actually satisfy a dependency and we won't ever
//!   try to activate a crate that's semver compatible with something else
//!   activated (as we're only allowed to have one) nor try to activate a crate
//!   that has the same links attribute as something else
//!   activated.
//! * Always try to activate the highest version crate first. The default
//!   dependency in Cargo (e.g., when you write `foo = "0.1.2"`) is
//!   semver-compatible, so selecting the highest version possible will allow us
//!   to hopefully satisfy as many dependencies at once.
//!
//! Beyond that, what's implemented below is just a naive backtracking version
//! which should in theory try all possible combinations of dependencies and
//! versions to see if one works. The first resolution that works causes
//! everything to bail out immediately and return success, and only if *nothing*
//! works do we actually return an error up the stack.
//!
//! ## Performance
//!
//! Note that this is a relatively performance-critical portion of Cargo. The
//! data that we're processing is proportional to the size of the dependency
//! graph, which can often be quite large (e.g., take a look at Servo). To make
//! matters worse the DFS algorithm we're implemented is inherently quite
//! inefficient. When we add the requirement of backtracking on top it means
//! that we're implementing something that probably shouldn't be allocating all
//! over the place.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::mem;
use std::rc::Rc;
use std::time::{Duration, Instant};

use log::{debug, trace};

use crate::core::interning::InternedString;
use crate::core::PackageIdSpec;
use crate::core::{Dependency, PackageId, Registry, Summary};
use crate::util::config::Config;
use crate::util::errors::CargoResult;
use crate::util::profile;

use self::context::{Activations, Context};
use self::types::{Candidate, ConflictMap, ConflictReason, DepsFrame, GraphNode};
use self::types::{RcVecIter, RegistryQueryer, RemainingDeps, ResolverProgress};

pub use self::encode::{EncodableDependency, EncodablePackageId, EncodableResolve};
pub use self::encode::{Metadata, WorkspaceResolve};
pub use self::errors::{ActivateError, ActivateResult, ResolveError};
pub use self::resolve::Resolve;
pub use self::types::Method;

mod conflict_cache;
mod context;
mod encode;
mod errors;
mod resolve;
mod types;

/// Builds the list of all packages required to build the first argument.
///
/// * `summaries` - the list of package summaries along with how to resolve
///   their features. This is a list of all top-level packages that are intended
///   to be part of the lock file (resolve output). These typically are a list
///   of all workspace members.
///
/// * `replacements` - this is a list of `[replace]` directives found in the
///   root of the workspace. The list here is a `PackageIdSpec` of what to
///   replace and a `Dependency` to replace that with. In general it's not
///   recommended to use `[replace]` any more and use `[patch]` instead, which
///   is supported elsewhere.
///
/// * `registry` - this is the source from which all package summaries are
///   loaded. It's expected that this is extensively configured ahead of time
///   and is idempotent with our requests to it (aka returns the same results
///   for the same query every time). Typically this is an instance of a
///   `PackageRegistry`.
///
/// * `try_to_use` - this is a list of package IDs which were previously found
///   in the lock file. We heuristically prefer the ids listed in `try_to_use`
///   when sorting candidates to activate, but otherwise this isn't used
///   anywhere else.
///
/// * `config` - a location to print warnings and such, or `None` if no warnings
///   should be printed
///
/// * `print_warnings` - whether or not to print backwards-compatibility
///   warnings and such
///
/// * `check_public_visible_dependencies` - a flag for whether to enforce the restrictions
///     introduced in the "public & private dependencies" RFC (1977). The current implementation
///     makes sure that there is only one version of each name visible to each package.
///
///     But there are 2 stable ways to directly depend on different versions of the same name.
///     1. Use the renamed dependencies functionality
///     2. Use 'cfg({})' dependencies functionality
///
///     When we have a decision for how to implement is without breaking existing functionality
///     this flag can be removed.
pub fn resolve(
    summaries: &[(Summary, Method<'_>)],
    replacements: &[(PackageIdSpec, Dependency)],
    registry: &mut dyn Registry,
    try_to_use: &HashSet<PackageId>,
    config: Option<&Config>,
    print_warnings: bool,
    check_public_visible_dependencies: bool,
) -> CargoResult<Resolve> {
    let cx = Context::new(check_public_visible_dependencies);
    let _p = profile::start("resolving");
    let minimal_versions = match config {
        Some(config) => config.cli_unstable().minimal_versions,
        None => false,
    };
    let mut registry = RegistryQueryer::new(registry, replacements, try_to_use, minimal_versions);
    let cx = activate_deps_loop(cx, &mut registry, summaries, config)?;

    let mut cksums = HashMap::new();
    for (summary, _) in cx.activations.values() {
        let cksum = summary.checksum().map(|s| s.to_string());
        cksums.insert(summary.package_id(), cksum);
    }
    let resolve = Resolve::new(
        cx.graph(),
        cx.resolve_replacements(),
        cx.resolve_features
            .iter()
            .map(|(k, v)| (*k, v.iter().map(|x| x.to_string()).collect()))
            .collect(),
        cksums,
        BTreeMap::new(),
        Vec::new(),
    );

    check_cycles(&resolve, &cx.activations)?;
    check_duplicate_pkgs_in_lockfile(&resolve)?;
    trace!("resolved: {:?}", resolve);

    // If we have a shell, emit warnings about required deps used as feature.
    if let Some(config) = config {
        if print_warnings {
            let mut shell = config.shell();
            let mut warnings = &cx.warnings;
            while let Some(ref head) = warnings.head {
                shell.warn(&head.0)?;
                warnings = &head.1;
            }
        }
    }

    Ok(resolve)
}

/// Recursively activates the dependencies for `top`, in depth-first order,
/// backtracking across possible candidates for each dependency as necessary.
///
/// If all dependencies can be activated and resolved to a version in the
/// dependency graph, cx.resolve is returned.
fn activate_deps_loop(
    mut cx: Context,
    registry: &mut RegistryQueryer<'_>,
    summaries: &[(Summary, Method<'_>)],
    config: Option<&Config>,
) -> CargoResult<Context> {
    let mut backtrack_stack = Vec::new();
    let mut remaining_deps = RemainingDeps::new();

    // `past_conflicting_activations` is a cache of the reasons for each time we
    // backtrack.
    let mut past_conflicting_activations = conflict_cache::ConflictCache::new();

    // Activate all the initial summaries to kick off some work.
    for &(ref summary, ref method) in summaries {
        debug!("initial activation: {}", summary.package_id());
        let candidate = Candidate {
            summary: summary.clone(),
            replace: None,
        };
        let res = activate(&mut cx, registry, None, candidate, method);
        match res {
            Ok(Some((frame, _))) => remaining_deps.push(frame),
            Ok(None) => (),
            Err(ActivateError::Fatal(e)) => return Err(e),
            Err(ActivateError::Conflict(_, _)) => panic!("bad error from activate"),
        }
    }

    let mut printed = ResolverProgress::new();

    // Main resolution loop, this is the workhorse of the resolution algorithm.
    //
    // You'll note that a few stacks are maintained on the side, which might
    // seem odd when this algorithm looks like it could be implemented
    // recursively. While correct, this is implemented iteratively to avoid
    // blowing the stack (the recursion depth is proportional to the size of the
    // input).
    //
    // The general sketch of this loop is to run until there are no dependencies
    // left to activate, and for each dependency to attempt to activate all of
    // its own dependencies in turn. The `backtrack_stack` is a side table of
    // backtracking states where if we hit an error we can return to in order to
    // attempt to continue resolving.
    while let Some((just_here_for_the_error_messages, frame)) =
        remaining_deps.pop_most_constrained()
    {
        let (mut parent, (mut cur, (mut dep, candidates, mut features))) = frame;

        // If we spend a lot of time here (we shouldn't in most cases) then give
        // a bit of a visual indicator as to what we're doing.
        printed.shell_status(config)?;

        trace!(
            "{}[{}]>{} {} candidates",
            parent.name(),
            cur,
            dep.package_name(),
            candidates.len()
        );

        let just_here_for_the_error_messages = just_here_for_the_error_messages
            && past_conflicting_activations
                .conflicting(&cx, &dep)
                .is_some();

        let mut remaining_candidates = RemainingCandidates::new(&candidates);

        // `conflicting_activations` stores all the reasons we were unable to
        // activate candidates. One of these reasons will have to go away for
        // backtracking to find a place to restart. It is also the list of
        // things to explain in the error message if we fail to resolve.
        //
        // This is a map of package ID to a reason why that packaged caused a
        // conflict for us.
        let mut conflicting_activations = ConflictMap::new();

        // When backtracking we don't fully update `conflicting_activations`
        // especially for the cases that we didn't make a backtrack frame in the
        // first place. This `backtracked` var stores whether we are continuing
        // from a restored backtrack frame so that we can skip caching
        // `conflicting_activations` in `past_conflicting_activations`
        let mut backtracked = false;

        loop {
            let next = remaining_candidates.next(
                &mut conflicting_activations,
                &cx,
                &dep,
                parent.package_id(),
            );

            let (candidate, has_another) = next.ok_or(()).or_else(|_| {
                // If we get here then our `remaining_candidates` was just
                // exhausted, so `dep` failed to activate.
                //
                // It's our job here to backtrack, if possible, and find a
                // different candidate to activate. If we can't find any
                // candidates whatsoever then it's time to bail entirely.
                trace!(
                    "{}[{}]>{} -- no candidates",
                    parent.name(),
                    cur,
                    dep.package_name()
                );

                // Use our list of `conflicting_activations` to add to our
                // global list of past conflicting activations, effectively
                // globally poisoning `dep` if `conflicting_activations` ever
                // shows up again. We'll use the `past_conflicting_activations`
                // below to determine if a dependency is poisoned and skip as
                // much work as possible.
                //
                // If we're only here for the error messages then there's no
                // need to try this as this dependency is already known to be
                // bad.
                //
                // As we mentioned above with the `backtracked` variable if this
                // local is set to `true` then our `conflicting_activations` may
                // not be right, so we can't push into our global cache.
                let mut generalize_conflicting_activations = None;
                if !just_here_for_the_error_messages && !backtracked {
                    past_conflicting_activations.insert(&dep, &conflicting_activations);
                    if let Some(c) = generalize_conflicting(
                        &cx,
                        registry,
                        &mut past_conflicting_activations,
                        &parent,
                        &dep,
                        &conflicting_activations,
                    ) {
                        generalize_conflicting_activations = Some(c);
                    }
                }

                match find_candidate(
                    &cx,
                    &mut backtrack_stack,
                    &parent,
                    backtracked,
                    generalize_conflicting_activations
                        .as_ref()
                        .unwrap_or(&conflicting_activations),
                ) {
                    Some((candidate, has_another, frame)) => {
                        // Reset all of our local variables used with the
                        // contents of `frame` to complete our backtrack.
                        cur = frame.cur;
                        cx = frame.context;
                        remaining_deps = frame.remaining_deps;
                        remaining_candidates = frame.remaining_candidates;
                        parent = frame.parent;
                        dep = frame.dep;
                        features = frame.features;
                        conflicting_activations = frame.conflicting_activations;
                        backtracked = true;
                        Ok((candidate, has_another))
                    }
                    None => {
                        debug!("no candidates found");
                        Err(errors::activation_error(
                            &cx,
                            registry.registry,
                            &parent,
                            &dep,
                            &conflicting_activations,
                            &candidates,
                            config,
                        ))
                    }
                }
            })?;

            // If we're only here for the error messages then we know that this
            // activation will fail one way or another. To that end if we've got
            // more candidates we want to fast-forward to the last one as
            // otherwise we'll just backtrack here anyway (helping us to skip
            // some work).
            if just_here_for_the_error_messages && !backtracked && has_another {
                continue;
            }

            // We have a `candidate`. Create a `BacktrackFrame` so we can add it
            // to the `backtrack_stack` later if activation succeeds.
            //
            // Note that if we don't actually have another candidate then there
            // will be nothing to backtrack to so we skip construction of the
            // frame. This is a relatively important optimization as a number of
            // the `clone` calls below can be quite expensive, so we avoid them
            // if we can.
            let backtrack = if has_another {
                Some(BacktrackFrame {
                    cur,
                    context: Context::clone(&cx),
                    remaining_deps: remaining_deps.clone(),
                    remaining_candidates: remaining_candidates.clone(),
                    parent: Summary::clone(&parent),
                    dep: Dependency::clone(&dep),
                    features: Rc::clone(&features),
                    conflicting_activations: conflicting_activations.clone(),
                })
            } else {
                None
            };

            let pid = candidate.summary.package_id();
            let method = Method::Required {
                dev_deps: false,
                features: &features,
                all_features: false,
                uses_default_features: dep.uses_default_features(),
            };
            trace!(
                "{}[{}]>{} trying {}",
                parent.name(),
                cur,
                dep.package_name(),
                candidate.summary.version()
            );
            let res = activate(&mut cx, registry, Some((&parent, &dep)), candidate, &method);

            let successfully_activated = match res {
                // Success! We've now activated our `candidate` in our context
                // and we're almost ready to move on. We may want to scrap this
                // frame in the end if it looks like it's not going to end well,
                // so figure that out here.
                Ok(Some((mut frame, dur))) => {
                    printed.elapsed(dur);

                    // Our `frame` here is a new package with its own list of
                    // dependencies. Do a sanity check here of all those
                    // dependencies by cross-referencing our global
                    // `past_conflicting_activations`. Recall that map is a
                    // global cache which lists sets of packages where, when
                    // activated, the dependency is unresolvable.
                    //
                    // If any our our frame's dependencies fit in that bucket,
                    // aka known unresolvable, then we extend our own set of
                    // conflicting activations with theirs. We can do this
                    // because the set of conflicts we found implies the
                    // dependency can't be activated which implies that we
                    // ourselves can't be activated, so we know that they
                    // conflict with us.
                    let mut has_past_conflicting_dep = just_here_for_the_error_messages;
                    if !has_past_conflicting_dep {
                        if let Some(conflicting) = frame
                            .remaining_siblings
                            .clone()
                            .filter_map(|(_, (ref new_dep, _, _))| {
                                past_conflicting_activations.conflicting(&cx, new_dep)
                            })
                            .next()
                        {
                            // If one of our deps is known unresolvable
                            // then we will not succeed.
                            // How ever if we are part of the reason that
                            // one of our deps conflicts then
                            // we can make a stronger statement
                            // because we will definitely be activated when
                            // we try our dep.
                            conflicting_activations.extend(
                                conflicting
                                    .iter()
                                    .filter(|&(p, _)| p != &pid)
                                    .map(|(&p, r)| (p, r.clone())),
                            );

                            has_past_conflicting_dep = true;
                        }
                    }
                    // If any of `remaining_deps` are known unresolvable with
                    // us activated, then we extend our own set of
                    // conflicting activations with theirs and its parent. We can do this
                    // because the set of conflicts we found implies the
                    // dependency can't be activated which implies that we
                    // ourselves are incompatible with that dep, so we know that deps
                    // parent conflict with us.
                    if !has_past_conflicting_dep {
                        if let Some(known_related_bad_deps) =
                            past_conflicting_activations.dependencies_conflicting_with(pid)
                        {
                            if let Some((other_parent, conflict)) = remaining_deps
                                .iter()
                                // for deps related to us
                                .filter(|&(_, ref other_dep)| {
                                    known_related_bad_deps.contains(other_dep)
                                })
                                .filter_map(|(other_parent, other_dep)| {
                                    past_conflicting_activations
                                        .find_conflicting(&cx, &other_dep, Some(pid))
                                        .map(|con| (other_parent, con))
                                })
                                .next()
                            {
                                let rel = conflict.get(&pid).unwrap().clone();

                                // The conflict we found is
                                // "other dep will not succeed if we are activated."
                                // We want to add
                                // "our dep will not succeed if other dep is in remaining_deps"
                                // but that is not how the cache is set up.
                                // So we add the less general but much faster,
                                // "our dep will not succeed if other dep's parent is activated".
                                conflicting_activations.extend(
                                    conflict
                                        .iter()
                                        .filter(|&(p, _)| p != &pid)
                                        .map(|(&p, r)| (p, r.clone())),
                                );
                                conflicting_activations.insert(other_parent, rel);
                                has_past_conflicting_dep = true;
                            }
                        }
                    }

                    // Ok if we're in a "known failure" state for this frame we
                    // may want to skip it altogether though. We don't want to
                    // skip it though in the case that we're displaying error
                    // messages to the user!
                    //
                    // Here we need to figure out if the user will see if we
                    // skipped this candidate (if it's known to fail, aka has a
                    // conflicting dep and we're the last candidate). If we're
                    // here for the error messages, we can't skip it (but we can
                    // prune extra work). If we don't have any candidates in our
                    // backtrack stack then we're the last line of defense, so
                    // we'll want to present an error message for sure.
                    let activate_for_error_message = has_past_conflicting_dep && !has_another && {
                        just_here_for_the_error_messages || {
                            find_candidate(
                                &cx,
                                &mut backtrack_stack.clone(),
                                &parent,
                                backtracked,
                                &conflicting_activations,
                            )
                            .is_none()
                        }
                    };

                    // If we're only here for the error messages then we know
                    // one of our candidate deps will fail, meaning we will
                    // fail and that none of the backtrack frames will find a
                    // candidate that will help. Consequently let's clean up the
                    // no longer needed backtrack frames.
                    if activate_for_error_message {
                        backtrack_stack.clear();
                    }

                    // If we don't know for a fact that we'll fail or if we're
                    // just here for the error message then we push this frame
                    // onto our list of to-be-resolve, which will generate more
                    // work for us later on.
                    //
                    // Otherwise we're guaranteed to fail and were not here for
                    // error messages, so we skip work and don't push anything
                    // onto our stack.
                    frame.just_for_error_messages = has_past_conflicting_dep;
                    if !has_past_conflicting_dep || activate_for_error_message {
                        remaining_deps.push(frame);
                        true
                    } else {
                        trace!(
                            "{}[{}]>{} skipping {} ",
                            parent.name(),
                            cur,
                            dep.package_name(),
                            pid.version()
                        );
                        false
                    }
                }

                // This candidate's already activated, so there's no extra work
                // for us to do. Let's keep going.
                Ok(None) => true,

                // We failed with a super fatal error (like a network error), so
                // bail out as quickly as possible as we can't reliably
                // backtrack from errors like these
                Err(ActivateError::Fatal(e)) => return Err(e),

                // We failed due to a bland conflict, bah! Record this in our
                // frame's list of conflicting activations as to why this
                // candidate failed, and then move on.
                Err(ActivateError::Conflict(id, reason)) => {
                    conflicting_activations.insert(id, reason);
                    false
                }
            };

            // If we've successfully activated then save off the backtrack frame
            // if one was created, and otherwise break out of the inner
            // activation loop as we're ready to move to the next dependency
            if successfully_activated {
                backtrack_stack.extend(backtrack);
                break;
            }

            // We've failed to activate this dependency, oh dear! Our call to
            // `activate` above may have altered our `cx` local variable, so
            // restore it back if we've got a backtrack frame.
            //
            // If we don't have a backtrack frame then we're just using the `cx`
            // for error messages anyway so we can live with a little
            // imprecision.
            if let Some(b) = backtrack {
                cx = b.context;
            }
        }

        // Ok phew, that loop was a big one! If we've broken out then we've
        // successfully activated a candidate. Our stacks are all in place that
        // we're ready to move on to the next dependency that needs activation,
        // so loop back to the top of the function here.
    }

    Ok(cx)
}

/// Attempts to activate the summary `candidate` in the context `cx`.
///
/// This function will pull dependency summaries from the registry provided, and
/// the dependencies of the package will be determined by the `method` provided.
/// If `candidate` was activated, this function returns the dependency frame to
/// iterate through next.
fn activate(
    cx: &mut Context,
    registry: &mut RegistryQueryer<'_>,
    parent: Option<(&Summary, &Dependency)>,
    candidate: Candidate,
    method: &Method<'_>,
) -> ActivateResult<Option<(DepsFrame, Duration)>> {
    let candidate_pid = candidate.summary.package_id();
    if let Some((parent, dep)) = parent {
        let parent_pid = parent.package_id();
        cx.resolve_graph
            .push(GraphNode::Link(parent_pid, candidate_pid, dep.clone()));
        Rc::make_mut(
            // add a edge from candidate to parent in the parents graph
            cx.parents.link(candidate_pid, parent_pid),
        )
        // and associate dep with that edge
        .push(dep.clone());
        if let Some(public_dependency) = cx.public_dependency.as_mut() {
            // one tricky part is that `candidate_pid` may already be active and
            // have public dependencies of its own. So we not only need to mark
            // `candidate_pid` as visible to its parents but also all of its existing
            // public dependencies.
            let existing_public_deps: Vec<PackageId> = public_dependency
                .get(&candidate_pid)
                .iter()
                .flat_map(|x| x.values())
                .filter_map(|x| if x.1 { Some(&x.0) } else { None })
                .chain(&Some(candidate_pid))
                .cloned()
                .collect();
            for c in existing_public_deps {
                // for each (transitive) parent that can newly see `t`
                let mut stack = vec![(parent_pid, dep.is_public())];
                while let Some((p, public)) = stack.pop() {
                    match public_dependency.entry(p).or_default().entry(c.name()) {
                        im_rc::hashmap::Entry::Occupied(mut o) => {
                            // the (transitive) parent can already see something by `c`s name, it had better be `c`.
                            assert_eq!(o.get().0, c);
                            if o.get().1 {
                                // The previous time the parent saw `c`, it was a public dependency.
                                // So all of its parents already know about `c`
                                // and we can save some time by stopping now.
                                continue;
                            }
                            if public {
                                // Mark that `c` has now bean seen publicly
                                o.insert((c, public));
                            }
                        }
                        im_rc::hashmap::Entry::Vacant(v) => {
                            // The (transitive) parent does not have anything by `c`s name,
                            // so we add `c`.
                            v.insert((c, public));
                        }
                    }
                    // if `candidate_pid` was a private dependency of `p` then `p` parents can't see `c` thru `p`
                    if public {
                        // if it was public, then we add all of `p`s parents to be checked
                        for &(grand, ref d) in cx.parents.edges(&p) {
                            stack.push((grand, d.iter().any(|x| x.is_public())));
                        }
                    }
                }
            }
        }
    }

    let activated = cx.flag_activated(&candidate.summary, method)?;

    let candidate = match candidate.replace {
        Some(replace) => {
            cx.resolve_replacements
                .push((candidate_pid, replace.package_id()));
            if cx.flag_activated(&replace, method)? && activated {
                return Ok(None);
            }
            trace!(
                "activating {} (replacing {})",
                replace.package_id(),
                candidate_pid
            );
            replace
        }
        None => {
            if activated {
                return Ok(None);
            }
            trace!("activating {}", candidate_pid);
            candidate.summary
        }
    };

    let now = Instant::now();
    let deps = cx.build_deps(registry, parent.map(|p| p.0), &candidate, method)?;
    let frame = DepsFrame {
        parent: candidate,
        just_for_error_messages: false,
        remaining_siblings: RcVecIter::new(Rc::new(deps)),
    };
    Ok(Some((frame, now.elapsed())))
}

#[derive(Clone)]
struct BacktrackFrame {
    cur: usize,
    context: Context,
    remaining_deps: RemainingDeps,
    remaining_candidates: RemainingCandidates,
    parent: Summary,
    dep: Dependency,
    features: Rc<Vec<InternedString>>,
    conflicting_activations: ConflictMap,
}

/// A helper "iterator" used to extract candidates within a current `Context` of
/// a dependency graph.
///
/// This struct doesn't literally implement the `Iterator` trait (requires a few
/// more inputs) but in general acts like one. Each `RemainingCandidates` is
/// created with a list of candidates to choose from. When attempting to iterate
/// over the list of candidates only *valid* candidates are returned. Validity
/// is defined within a `Context`.
///
/// Candidates passed to `new` may not be returned from `next` as they could be
/// filtered out, and as they are filtered the causes will be added to `conflicting_prev_active`.
#[derive(Clone)]
struct RemainingCandidates {
    remaining: RcVecIter<Candidate>,
    // This is a inlined peekable generator
    has_another: Option<Candidate>,
}

impl RemainingCandidates {
    fn new(candidates: &Rc<Vec<Candidate>>) -> RemainingCandidates {
        RemainingCandidates {
            remaining: RcVecIter::new(Rc::clone(candidates)),
            has_another: None,
        }
    }

    /// Attempts to find another candidate to check from this list.
    ///
    /// This method will attempt to move this iterator forward, returning a
    /// candidate that's possible to activate. The `cx` argument is the current
    /// context which determines validity for candidates returned, and the `dep`
    /// is the dependency listing that we're activating for.
    ///
    /// If successful a `(Candidate, bool)` pair will be returned. The
    /// `Candidate` is the candidate to attempt to activate, and the `bool` is
    /// an indicator of whether there are remaining candidates to try of if
    /// we've reached the end of iteration.
    ///
    /// If we've reached the end of the iterator here then `Err` will be
    /// returned. The error will contain a map of package ID to conflict reason,
    /// where each package ID caused a candidate to be filtered out from the
    /// original list for the reason listed.
    fn next(
        &mut self,
        conflicting_prev_active: &mut ConflictMap,
        cx: &Context,
        dep: &Dependency,
        parent: PackageId,
    ) -> Option<(Candidate, bool)> {
        'main: for (_, b) in self.remaining.by_ref() {
            let b_id = b.summary.package_id();
            // The `links` key in the manifest dictates that there's only one
            // package in a dependency graph, globally, with that particular
            // `links` key. If this candidate links to something that's already
            // linked to by a different package then we've gotta skip this.
            if let Some(link) = b.summary.links() {
                if let Some(&a) = cx.links.get(&link) {
                    if a != b_id {
                        conflicting_prev_active
                            .entry(a)
                            .or_insert_with(|| ConflictReason::Links(link));
                        continue;
                    }
                }
            }

            // Otherwise the condition for being a valid candidate relies on
            // semver. Cargo dictates that you can't duplicate multiple
            // semver-compatible versions of a crate. For example we can't
            // simultaneously activate `foo 1.0.2` and `foo 1.2.0`. We can,
            // however, activate `1.0.2` and `2.0.0`.
            //
            // Here we throw out our candidate if it's *compatible*, yet not
            // equal, to all previously activated versions.
            if let Some((a, _)) = cx.activations.get(&b_id.as_activations_key()) {
                if *a != b.summary {
                    conflicting_prev_active
                        .entry(a.package_id())
                        .or_insert(ConflictReason::Semver);
                    continue;
                }
            }
            // We may still have to reject do to a public dependency conflict. If one of any of our
            // ancestors that can see us already knows about a different crate with this name then
            // we have to reject this candidate. Additionally this candidate may already have been
            // activated and have public dependants of its own,
            // all of witch also need to be checked the same way.
            if let Some(public_dependency) = cx.public_dependency.as_ref() {
                let existing_public_deps: Vec<PackageId> = public_dependency
                    .get(&b_id)
                    .iter()
                    .flat_map(|x| x.values())
                    .filter_map(|x| if x.1 { Some(&x.0) } else { None })
                    .chain(&Some(b_id))
                    .cloned()
                    .collect();
                for t in existing_public_deps {
                    // for each (transitive) parent that can newly see `t`
                    let mut stack = vec![(parent, dep.is_public())];
                    while let Some((p, public)) = stack.pop() {
                        // TODO: dont look at the same thing more then once
                        if let Some(o) = public_dependency.get(&p).and_then(|x| x.get(&t.name())) {
                            if o.0 != t {
                                // the (transitive) parent can already see a different version by `t`s name.
                                // So, adding `b` will cause `p` to have a public dependency conflict on `t`.
                                conflicting_prev_active.insert(p, ConflictReason::PublicDependency);
                                continue 'main;
                            }
                        }
                        // if `b` was a private dependency of `p` then `p` parents can't see `t` thru `p`
                        if public {
                            // if it was public, then we add all of `p`s parents to be checked
                            for &(grand, ref d) in cx.parents.edges(&p) {
                                stack.push((grand, d.iter().any(|x| x.is_public())));
                            }
                        }
                    }
                }
            }

            // Well if we made it this far then we've got a valid dependency. We
            // want this iterator to be inherently "peekable" so we don't
            // necessarily return the item just yet. Instead we stash it away to
            // get returned later, and if we replaced something then that was
            // actually the candidate to try first so we return that.
            if let Some(r) = mem::replace(&mut self.has_another, Some(b)) {
                return Some((r, true));
            }
        }

        // Alright we've entirely exhausted our list of candidates. If we've got
        // something stashed away return that here (also indicating that there's
        // nothing else).
        self.has_another.take().map(|r| (r, false))
    }
}

/// Attempts to find a new conflict that allows a bigger backjump then the input one.
/// It will add the new conflict to the cache if one is found.
///
/// Panics if the input conflict is not all active in `cx`.
fn generalize_conflicting(
    cx: &Context,
    registry: &mut RegistryQueryer<'_>,
    past_conflicting_activations: &mut conflict_cache::ConflictCache,
    parent: &Summary,
    dep: &Dependency,
    conflicting_activations: &ConflictMap,
) -> Option<ConflictMap> {
    if conflicting_activations.is_empty() {
        return None;
    }
    // We need to determine the "age" that this `conflicting_activations` will jump to, and why.
    let (jumpback_critical_age, jumpback_critical_id) = conflicting_activations
        .keys()
        .map(|&c| (cx.is_active(c).expect("not currently active!?"), c))
        .max()
        .unwrap();
    let jumpback_critical_reason: ConflictReason =
        conflicting_activations[&jumpback_critical_id].clone();

    if cx
        .parents
        .is_path_from_to(&parent.package_id(), &jumpback_critical_id)
    {
        // We are a descendant of the trigger of the problem.
        // The best generalization of this is to let things bubble up
        // and let `jumpback_critical_id` figure this out.
        return None;
    }
    // What parents does that critical activation have
    for (critical_parent, critical_parents_deps) in
        cx.parents.edges(&jumpback_critical_id).filter(|(p, _)| {
            // it will only help backjump further if it is older then the critical_age
            cx.is_active(*p).expect("parent not currently active!?") < jumpback_critical_age
        })
    {
        for critical_parents_dep in critical_parents_deps.iter() {
            // A dep is equivalent to one of the things it can resolve to.
            // Thus, if all the things it can resolve to have already ben determined
            // to be conflicting, then we can just say that we conflict with the parent.
            if registry
                .query(&critical_parents_dep)
                .expect("an already used dep now error!?")
                .iter()
                .rev() // the last one to be tried is the least likely to be in the cache, so start with that.
                .all(|other| {
                    let mut con = conflicting_activations.clone();
                    con.remove(&jumpback_critical_id);
                    con.insert(other.summary.package_id(), jumpback_critical_reason.clone());
                    past_conflicting_activations.contains(&dep, &con)
                })
            {
                let mut con = conflicting_activations.clone();
                con.remove(&jumpback_critical_id);
                con.insert(*critical_parent, jumpback_critical_reason);
                past_conflicting_activations.insert(&dep, &con);
                return Some(con);
            }
        }
    }
    None
}

/// Looks through the states in `backtrack_stack` for dependencies with
/// remaining candidates. For each one, also checks if rolling back
/// could change the outcome of the failed resolution that caused backtracking
/// in the first place. Namely, if we've backtracked past the parent of the
/// failed dep, or any of the packages flagged as giving us trouble in
/// `conflicting_activations`.
///
/// Read <https://github.com/rust-lang/cargo/pull/4834>
/// For several more detailed explanations of the logic here.
fn find_candidate(
    cx: &Context,
    backtrack_stack: &mut Vec<BacktrackFrame>,
    parent: &Summary,
    backtracked: bool,
    conflicting_activations: &ConflictMap,
) -> Option<(Candidate, bool, BacktrackFrame)> {
    // When we're calling this method we know that `parent` failed to
    // activate. That means that some dependency failed to get resolved for
    // whatever reason. Normally, that means that all of those reasons
    // (plus maybe some extras) are listed in `conflicting_activations`.
    //
    // The abnormal situations are things that do not put all of the reasons in `conflicting_activations`:
    // If we backtracked we do not know how our `conflicting_activations` related to
    // the cause of that backtrack, so we do not update it.
    // If we had a PublicDependency conflict, then we do not yet have a compact way to
    // represent all the parts of the problem, so `conflicting_activations` is incomplete.
    let age = if !backtracked
        && !conflicting_activations
            .values()
            .any(|c| *c == ConflictReason::PublicDependency)
    {
        // we dont have abnormal situations. So we can ask `cx` for how far back we need to go.
        cx.is_conflicting(Some(parent.package_id()), conflicting_activations)
    } else {
        None
    };

    while let Some(mut frame) = backtrack_stack.pop() {
        let next = frame.remaining_candidates.next(
            &mut frame.conflicting_activations,
            &frame.context,
            &frame.dep,
            frame.parent.package_id(),
        );
        let (candidate, has_another) = match next {
            Some(pair) => pair,
            None => continue,
        };

        // If all members of `conflicting_activations` are still
        // active in this back up we know that we're guaranteed to not actually
        // make any progress. As a result if we hit this condition we can
        // completely skip this backtrack frame and move on to the next.
        if let Some(age) = age {
            if frame.context.activations.len() > age {
                trace!(
                    "{} = \"{}\" skip as not solving {}: {:?}",
                    frame.dep.package_name(),
                    frame.dep.version_req(),
                    parent.package_id(),
                    conflicting_activations
                );
                // above we use `cx` to determine that this is still going to be conflicting.
                // but lets just double check.
                debug_assert!(
                    frame
                        .context
                        .is_conflicting(Some(parent.package_id()), conflicting_activations)
                        == Some(age)
                );
                continue;
            } else {
                // above we use `cx` to determine that this is not going to be conflicting.
                // but lets just double check.
                debug_assert!(frame
                    .context
                    .is_conflicting(Some(parent.package_id()), conflicting_activations)
                    .is_none());
            }
        }

        return Some((candidate, has_another, frame));
    }
    None
}

fn check_cycles(resolve: &Resolve, activations: &Activations) -> CargoResult<()> {
    let summaries: HashMap<PackageId, &Summary> = activations
        .values()
        .map(|(s, _)| (s.package_id(), s))
        .collect();

    // Sort packages to produce user friendly deterministic errors.
    let mut all_packages: Vec<_> = resolve.iter().collect();
    all_packages.sort_unstable();
    let mut checked = HashSet::new();
    for pkg in all_packages {
        if !checked.contains(&pkg) {
            visit(resolve, pkg, &summaries, &mut HashSet::new(), &mut checked)?
        }
    }
    return Ok(());

    fn visit(
        resolve: &Resolve,
        id: PackageId,
        summaries: &HashMap<PackageId, &Summary>,
        visited: &mut HashSet<PackageId>,
        checked: &mut HashSet<PackageId>,
    ) -> CargoResult<()> {
        // See if we visited ourselves
        if !visited.insert(id) {
            failure::bail!(
                "cyclic package dependency: package `{}` depends on itself. Cycle:\n{}",
                id,
                errors::describe_path(&resolve.path_to_top(&id))
            );
        }

        // If we've already checked this node no need to recurse again as we'll
        // just conclude the same thing as last time, so we only execute the
        // recursive step if we successfully insert into `checked`.
        //
        // Note that if we hit an intransitive dependency then we clear out the
        // visitation list as we can't induce a cycle through transitive
        // dependencies.
        if checked.insert(id) {
            let summary = summaries[&id];
            for dep in resolve.deps_not_replaced(id) {
                let is_transitive = summary
                    .dependencies()
                    .iter()
                    .any(|d| d.matches_id(dep) && d.is_transitive());
                let mut empty = HashSet::new();
                let visited = if is_transitive {
                    &mut *visited
                } else {
                    &mut empty
                };
                visit(resolve, dep, summaries, visited, checked)?;

                if let Some(id) = resolve.replacement(dep) {
                    visit(resolve, id, summaries, visited, checked)?;
                }
            }
        }

        // Ok, we're done, no longer visiting our node any more
        visited.remove(&id);
        Ok(())
    }
}

/// Checks that packages are unique when written to lock file.
///
/// When writing package ID's to lock file, we apply lossy encoding. In
/// particular, we don't store paths of path dependencies. That means that
/// *different* packages may collide in the lock file, hence this check.
fn check_duplicate_pkgs_in_lockfile(resolve: &Resolve) -> CargoResult<()> {
    let mut unique_pkg_ids = HashMap::new();
    for pkg_id in resolve.iter() {
        let encodable_pkd_id = encode::encodable_package_id(pkg_id);
        if let Some(prev_pkg_id) = unique_pkg_ids.insert(encodable_pkd_id, pkg_id) {
            failure::bail!(
                "package collision in the lockfile: packages {} and {} are different, \
                 but only one can be written to lockfile unambiguously",
                prev_pkg_id,
                pkg_id
            )
        }
    }
    Ok(())
}
