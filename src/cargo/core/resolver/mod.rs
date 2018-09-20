//! Resolution of the entire dependency graph for a crate
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
//! 2. Crates can be activated with multiple version (e.g. show up in the
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
//!   dependency in Cargo (e.g. when you write `foo = "0.1.2"`) is
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
//! graph, which can often be quite large (e.g. take a look at Servo). To make
//! matters worse the DFS algorithm we're implemented is inherently quite
//! inefficient. When we add the requirement of backtracking on top it means
//! that we're implementing something that probably shouldn't be allocating all
//! over the place.

use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::mem;
use std::rc::Rc;
use std::time::{Duration, Instant};

use semver;

use core::interning::InternedString;
use core::PackageIdSpec;
use core::{Dependency, PackageId, Registry, Summary};
use util::config::Config;
use util::errors::{CargoError, CargoResult};
use util::lev_distance::lev_distance;
use util::profile;

use self::context::{Activations, Context};
use self::types::{ActivateError, ActivateResult, Candidate, ConflictReason, DepsFrame, GraphNode};
use self::types::{RcVecIter, RegistryQueryer};

pub use self::encode::{EncodableDependency, EncodablePackageId, EncodableResolve};
pub use self::encode::{Metadata, WorkspaceResolve};
pub use self::resolve::{Deps, DepsNotReplaced, Resolve};
pub use self::types::Method;

mod conflict_cache;
mod context;
mod encode;
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
/// * `try_to_use` - this is a list of package ids which were previously found
///   in the lock file. We heuristically prefer the ids listed in `try_to_use`
///   when sorting candidates to activate, but otherwise this isn't used
///   anywhere else.
///
/// * `config` - a location to print warnings and such, or `None` if no warnings
///   should be printed
///
/// * `print_warnings` - whether or not to print backwards-compatibility
///   warnings and such
pub fn resolve(
    summaries: &[(Summary, Method)],
    replacements: &[(PackageIdSpec, Dependency)],
    registry: &mut Registry,
    try_to_use: &HashSet<&PackageId>,
    config: Option<&Config>,
    print_warnings: bool,
) -> CargoResult<Resolve> {
    let cx = Context::new();
    let _p = profile::start("resolving");
    let minimal_versions = match config {
        Some(config) => config.cli_unstable().minimal_versions,
        None => false,
    };
    let mut registry = RegistryQueryer::new(registry, replacements, try_to_use, minimal_versions);
    let cx = activate_deps_loop(cx, &mut registry, summaries, config)?;

    let mut cksums = HashMap::new();
    for summary in cx.activations.values().flat_map(|v| v.iter()) {
        let cksum = summary.checksum().map(|s| s.to_string());
        cksums.insert(summary.package_id().clone(), cksum);
    }
    let resolve = Resolve::new(
        cx.graph(),
        cx.resolve_replacements(),
        cx.resolve_features
            .iter()
            .map(|(k, v)| (k.clone(), v.iter().map(|x| x.to_string()).collect()))
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
    registry: &mut RegistryQueryer,
    summaries: &[(Summary, Method)],
    config: Option<&Config>,
) -> CargoResult<Context> {
    // Note that a `BinaryHeap` is used for the remaining dependencies that need
    // activation. This heap is sorted such that the "largest value" is the most
    // constrained dependency, or the one with the least candidates.
    //
    // This helps us get through super constrained portions of the dependency
    // graph quickly and hopefully lock down what later larger dependencies can
    // use (those with more candidates).
    let mut backtrack_stack = Vec::new();
    let mut remaining_deps = BinaryHeap::new();

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

    let mut ticks = 0u16;
    let start = Instant::now();
    let time_to_print = Duration::from_millis(500);
    let mut printed = false;
    let mut deps_time = Duration::new(0, 0);

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
    while let Some(mut deps_frame) = remaining_deps.pop() {
        // If we spend a lot of time here (we shouldn't in most cases) then give
        // a bit of a visual indicator as to what we're doing. Only enable this
        // when stderr is a tty (a human is likely to be watching) to ensure we
        // get deterministic output otherwise when observed by tools.
        //
        // Also note that we hit this loop a lot, so it's fairly performance
        // sensitive. As a result try to defer a possibly expensive operation
        // like `Instant::now` by only checking every N iterations of this loop
        // to amortize the cost of the current time lookup.
        ticks += 1;
        if let Some(config) = config {
            if config.shell().is_err_tty()
                && !printed
                && ticks % 1000 == 0
                && start.elapsed() - deps_time > time_to_print
            {
                printed = true;
                config.shell().status("Resolving", "dependency graph...")?;
            }
        }
        // The largest test in our sweet takes less then 5000 ticks
        // with all the algorithm improvements.
        // If any of them are removed then it takes more than I am willing to measure.
        // So lets fail the test fast if we have ben running for two long.
        debug_assert!(ticks < 50_000);
        // The largest test in our sweet takes less then 30 sec
        // with all the improvements to how fast a tick can go.
        // If any of them are removed then it takes more than I am willing to measure.
        // So lets fail the test fast if we have ben running for two long.
        if cfg!(debug_assertions) && (ticks % 1000 == 0) {
            assert!(start.elapsed() - deps_time < Duration::from_secs(300));
        }

        let just_here_for_the_error_messages = deps_frame.just_for_error_messages;

        // Figure out what our next dependency to activate is, and if nothing is
        // listed then we're entirely done with this frame (yay!) and we can
        // move on to the next frame.
        let frame = match deps_frame.remaining_siblings.next() {
            Some(sibling) => {
                let parent = Summary::clone(&deps_frame.parent);
                remaining_deps.push(deps_frame);
                (parent, sibling)
            }
            None => continue,
        };
        let (mut parent, (mut cur, (mut dep, candidates, mut features))) = frame;
        assert!(!remaining_deps.is_empty());

        trace!(
            "{}[{}]>{} {} candidates",
            parent.name(),
            cur,
            dep.package_name(),
            candidates.len()
        );
        trace!(
            "{}[{}]>{} {} prev activations",
            parent.name(),
            cur,
            dep.package_name(),
            cx.prev_active(&dep).len()
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
        // This is a map of package id to a reason why that packaged caused a
        // conflict for us.
        let mut conflicting_activations = HashMap::new();

        // When backtracking we don't fully update `conflicting_activations`
        // especially for the cases that we didn't make a backtrack frame in the
        // first place.  This `backtracked` var stores whether we are continuing
        // from a restored backtrack frame so that we can skip caching
        // `conflicting_activations` in `past_conflicting_activations`
        let mut backtracked = false;

        loop {
            let next = remaining_candidates.next(&mut conflicting_activations, &cx, &dep);

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
                if !just_here_for_the_error_messages && !backtracked {
                    past_conflicting_activations.insert(&dep, &conflicting_activations);
                }

                match find_candidate(
                    &mut backtrack_stack,
                    &parent,
                    backtracked,
                    &conflicting_activations,
                ) {
                    Some((candidate, has_another, frame)) => {
                        // Reset all of our local variables used with the
                        // contents of `frame` to complete our backtrack.
                        cur = frame.cur;
                        cx = frame.context_backup;
                        remaining_deps = frame.deps_backup;
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
                        Err(activation_error(
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
                    context_backup: Context::clone(&cx),
                    deps_backup: <BinaryHeap<DepsFrame>>::clone(&remaining_deps),
                    remaining_candidates: remaining_candidates.clone(),
                    parent: Summary::clone(&parent),
                    dep: Dependency::clone(&dep),
                    features: Rc::clone(&features),
                    conflicting_activations: conflicting_activations.clone(),
                })
            } else {
                None
            };

            let pid = candidate.summary.package_id().clone();
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
                    deps_time += dur;

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
                            }).next()
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
                                    .map(|(p, r)| (p.clone(), r.clone())),
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
                            past_conflicting_activations.dependencies_conflicting_with(&pid)
                        {
                            if let Some((other_parent, conflict)) = remaining_deps
                                .iter()
                                .flat_map(|other| other.flatten())
                                // for deps related to us
                                .filter(|&(_, ref other_dep)| {
                                    known_related_bad_deps.contains(other_dep)
                                }).filter_map(|(other_parent, other_dep)| {
                                    past_conflicting_activations
                                        .find_conflicting(&cx, &other_dep, |con| {
                                            con.contains_key(&pid)
                                        }).map(|con| (other_parent, con))
                                }).next()
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
                                        .map(|(p, r)| (p.clone(), r.clone())),
                                );
                                conflicting_activations.insert(other_parent.clone(), rel);
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
                                &mut backtrack_stack.clone(),
                                &parent,
                                backtracked,
                                &conflicting_activations,
                            ).is_none()
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
                cx = b.context_backup;
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
    registry: &mut RegistryQueryer,
    parent: Option<(&Summary, &Dependency)>,
    candidate: Candidate,
    method: &Method,
) -> ActivateResult<Option<(DepsFrame, Duration)>> {
    if let Some((parent, dep)) = parent {
        cx.resolve_graph.push(GraphNode::Link(
            parent.package_id().clone(),
            candidate.summary.package_id().clone(),
            dep.clone(),
        ));
    }

    let activated = cx.flag_activated(&candidate.summary, method)?;

    let candidate = match candidate.replace {
        Some(replace) => {
            cx.resolve_replacements.push((
                candidate.summary.package_id().clone(),
                replace.package_id().clone(),
            ));
            if cx.flag_activated(&replace, method)? && activated {
                return Ok(None);
            }
            trace!(
                "activating {} (replacing {})",
                replace.package_id(),
                candidate.summary.package_id()
            );
            replace
        }
        None => {
            if activated {
                return Ok(None);
            }
            trace!("activating {}", candidate.summary.package_id());
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
    context_backup: Context,
    deps_backup: BinaryHeap<DepsFrame>,
    remaining_candidates: RemainingCandidates,
    parent: Summary,
    dep: Dependency,
    features: Rc<Vec<InternedString>>,
    conflicting_activations: HashMap<PackageId, ConflictReason>,
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
    /// returned. The error will contain a map of package id to conflict reason,
    /// where each package id caused a candidate to be filtered out from the
    /// original list for the reason listed.
    fn next(
        &mut self,
        conflicting_prev_active: &mut HashMap<PackageId, ConflictReason>,
        cx: &Context,
        dep: &Dependency,
    ) -> Option<(Candidate, bool)> {
        let prev_active = cx.prev_active(dep);

        for (_, b) in self.remaining.by_ref() {
            // The `links` key in the manifest dictates that there's only one
            // package in a dependency graph, globally, with that particular
            // `links` key. If this candidate links to something that's already
            // linked to by a different package then we've gotta skip this.
            if let Some(link) = b.summary.links() {
                if let Some(a) = cx.links.get(&link) {
                    if a != b.summary.package_id() {
                        conflicting_prev_active
                            .entry(a.clone())
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
            if let Some(a) = prev_active
                .iter()
                .find(|a| compatible(a.version(), b.summary.version()))
            {
                if *a != b.summary {
                    conflicting_prev_active
                        .entry(a.package_id().clone())
                        .or_insert(ConflictReason::Semver);
                    continue;
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

// Returns if `a` and `b` are compatible in the semver sense. This is a
// commutative operation.
//
// Versions `a` and `b` are compatible if their left-most nonzero digit is the
// same.
fn compatible(a: &semver::Version, b: &semver::Version) -> bool {
    if a.major != b.major {
        return false;
    }
    if a.major != 0 {
        return true;
    }
    if a.minor != b.minor {
        return false;
    }
    if a.minor != 0 {
        return true;
    }
    a.patch == b.patch
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
    backtrack_stack: &mut Vec<BacktrackFrame>,
    parent: &Summary,
    backtracked: bool,
    conflicting_activations: &HashMap<PackageId, ConflictReason>,
) -> Option<(Candidate, bool, BacktrackFrame)> {
    while let Some(mut frame) = backtrack_stack.pop() {
        let next = frame.remaining_candidates.next(
            &mut frame.conflicting_activations,
            &frame.context_backup,
            &frame.dep,
        );
        let (candidate, has_another) = match next {
            Some(pair) => pair,
            None => continue,
        };
        // When we're calling this method we know that `parent` failed to
        // activate. That means that some dependency failed to get resolved for
        // whatever reason, and all of those reasons (plus maybe some extras)
        // are listed in `conflicting_activations`.
        //
        // This means that if all members of `conflicting_activations` are still
        // active in this back up we know that we're guaranteed to not actually
        // make any progress. As a result if we hit this condition we can
        // completely skip this backtrack frame and move on to the next.
        if !backtracked {
            if frame
                .context_backup
                .is_conflicting(Some(parent.package_id()), conflicting_activations)
            {
                trace!(
                    "{} = \"{}\" skip as not solving {}: {:?}",
                    frame.dep.package_name(),
                    frame.dep.version_req(),
                    parent.package_id(),
                    conflicting_activations
                );
                continue;
            }
        }

        return Some((candidate, has_another, frame));
    }
    None
}

fn activation_error(
    cx: &Context,
    registry: &mut Registry,
    parent: &Summary,
    dep: &Dependency,
    conflicting_activations: &HashMap<PackageId, ConflictReason>,
    candidates: &[Candidate],
    config: Option<&Config>,
) -> CargoError {
    let graph = cx.graph();
    if !candidates.is_empty() {
        let mut msg = format!("failed to select a version for `{}`.", dep.package_name());
        msg.push_str("\n    ... required by ");
        msg.push_str(&describe_path(&graph.path_to_top(parent.package_id())));

        msg.push_str("\nversions that meet the requirements `");
        msg.push_str(&dep.version_req().to_string());
        msg.push_str("` are: ");
        msg.push_str(
            &candidates
                .iter()
                .map(|v| v.summary.version())
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );

        let mut conflicting_activations: Vec<_> = conflicting_activations.iter().collect();
        conflicting_activations.sort_unstable();
        let (links_errors, mut other_errors): (Vec<_>, Vec<_>) = conflicting_activations
            .drain(..)
            .rev()
            .partition(|&(_, r)| r.is_links());

        for &(p, r) in links_errors.iter() {
            if let ConflictReason::Links(ref link) = *r {
                msg.push_str("\n\nthe package `");
                msg.push_str(&*dep.package_name());
                msg.push_str("` links to the native library `");
                msg.push_str(link);
                msg.push_str("`, but it conflicts with a previous package which links to `");
                msg.push_str(link);
                msg.push_str("` as well:\n");
            }
            msg.push_str(&describe_path(&graph.path_to_top(p)));
        }

        let (features_errors, other_errors): (Vec<_>, Vec<_>) = other_errors
            .drain(..)
            .partition(|&(_, r)| r.is_missing_features());

        for &(p, r) in features_errors.iter() {
            if let ConflictReason::MissingFeatures(ref features) = *r {
                msg.push_str("\n\nthe package `");
                msg.push_str(&*p.name());
                msg.push_str("` depends on `");
                msg.push_str(&*dep.package_name());
                msg.push_str("`, with features: `");
                msg.push_str(features);
                msg.push_str("` but `");
                msg.push_str(&*dep.package_name());
                msg.push_str("` does not have these features.\n");
            }
            // p == parent so the full path is redundant.
        }

        if !other_errors.is_empty() {
            msg.push_str(
                "\n\nall possible versions conflict with \
                 previously selected packages.",
            );
        }

        for &(p, _) in other_errors.iter() {
            msg.push_str("\n\n  previously selected ");
            msg.push_str(&describe_path(&graph.path_to_top(p)));
        }

        msg.push_str("\n\nfailed to select a version for `");
        msg.push_str(&*dep.package_name());
        msg.push_str("` which could resolve this conflict");

        return format_err!("{}", msg);
    }

    // We didn't actually find any candidates, so we need to
    // give an error message that nothing was found.
    //
    // Maybe the user mistyped the ver_req? Like `dep="2"` when `dep="0.2"`
    // was meant. So we re-query the registry with `deb="*"` so we can
    // list a few versions that were actually found.
    let all_req = semver::VersionReq::parse("*").unwrap();
    let mut new_dep = dep.clone();
    new_dep.set_version_req(all_req);
    let mut candidates = match registry.query_vec(&new_dep, false) {
        Ok(candidates) => candidates,
        Err(e) => return e,
    };
    candidates.sort_unstable_by(|a, b| b.version().cmp(a.version()));

    let mut msg = if !candidates.is_empty() {
        let versions = {
            let mut versions = candidates
                .iter()
                .take(3)
                .map(|cand| cand.version().to_string())
                .collect::<Vec<_>>();

            if candidates.len() > 3 {
                versions.push("...".into());
            }

            versions.join(", ")
        };

        let mut msg = format!(
            "no matching version `{}` found for package `{}`\n\
             location searched: {}\n\
             versions found: {}\n",
            dep.version_req(),
            dep.package_name(),
            dep.source_id(),
            versions
        );
        msg.push_str("required by ");
        msg.push_str(&describe_path(&graph.path_to_top(parent.package_id())));

        // If we have a path dependency with a locked version, then this may
        // indicate that we updated a sub-package and forgot to run `cargo
        // update`. In this case try to print a helpful error!
        if dep.source_id().is_path() && dep.version_req().to_string().starts_with('=') {
            msg.push_str(
                "\nconsider running `cargo update` to update \
                 a path dependency's locked version",
            );
        }

        msg
    } else {
        // Maybe the user mistyped the name? Like `dep-thing` when `Dep_Thing`
        // was meant. So we try asking the registry for a `fuzzy` search for suggestions.
        let mut candidates = Vec::new();
        if let Err(e) = registry.query(&new_dep, &mut |s| candidates.push(s.name()), true) {
            return e;
        };
        candidates.sort_unstable();
        candidates.dedup();
        let mut candidates: Vec<_> = candidates
            .iter()
            .map(|n| (lev_distance(&*new_dep.package_name(), &*n), n))
            .filter(|&(d, _)| d < 4)
            .collect();
        candidates.sort_by_key(|o| o.0);
        let mut msg = format!(
            "no matching package named `{}` found\n\
             location searched: {}\n",
            dep.package_name(),
            dep.source_id()
        );
        if !candidates.is_empty() {
            let mut names = candidates
                .iter()
                .take(3)
                .map(|c| c.1.as_str())
                .collect::<Vec<_>>();

            if candidates.len() > 3 {
                names.push("...");
            }

            msg.push_str("did you mean: ");
            msg.push_str(&names.join(", "));
            msg.push_str("\n");
        }
        msg.push_str("required by ");
        msg.push_str(&describe_path(&graph.path_to_top(parent.package_id())));

        msg
    };

    if let Some(config) = config {
        if config.cli_unstable().offline {
            msg.push_str(
                "\nAs a reminder, you're using offline mode (-Z offline) \
                 which can sometimes cause surprising resolution failures, \
                 if this error is too confusing you may with to retry \
                 without the offline flag.",
            );
        }
    }

    format_err!("{}", msg)
}

/// Returns String representation of dependency chain for a particular `pkgid`.
fn describe_path(path: &[&PackageId]) -> String {
    use std::fmt::Write;
    let mut dep_path_desc = format!("package `{}`", path[0]);
    for dep in path[1..].iter() {
        write!(dep_path_desc, "\n    ... which is depended on by `{}`", dep).unwrap();
    }
    dep_path_desc
}

fn check_cycles(resolve: &Resolve, activations: &Activations) -> CargoResult<()> {
    let summaries: HashMap<&PackageId, &Summary> = activations
        .values()
        .flat_map(|v| v.iter())
        .map(|s| (s.package_id(), s))
        .collect();

    // Sort packages to produce user friendly deterministic errors.
    let mut all_packages: Vec<_> = resolve.iter().collect();
    all_packages.sort_unstable();
    let mut checked = HashSet::new();
    for pkg in all_packages {
        if !checked.contains(pkg) {
            visit(resolve, pkg, &summaries, &mut HashSet::new(), &mut checked)?
        }
    }
    return Ok(());

    fn visit<'a>(
        resolve: &'a Resolve,
        id: &'a PackageId,
        summaries: &HashMap<&'a PackageId, &Summary>,
        visited: &mut HashSet<&'a PackageId>,
        checked: &mut HashSet<&'a PackageId>,
    ) -> CargoResult<()> {
        // See if we visited ourselves
        if !visited.insert(id) {
            bail!(
                "cyclic package dependency: package `{}` depends on itself. Cycle:\n{}",
                id,
                describe_path(&resolve.path_to_top(id))
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
            let summary = summaries[id];
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
        visited.remove(id);
        Ok(())
    }
}

/// Checks that packages are unique when written to lockfile.
///
/// When writing package id's to lockfile, we apply lossy encoding. In
/// particular, we don't store paths of path dependencies. That means that
/// *different* packages may collide in the lockfile, hence this check.
fn check_duplicate_pkgs_in_lockfile(resolve: &Resolve) -> CargoResult<()> {
    let mut unique_pkg_ids = HashMap::new();
    for pkg_id in resolve.iter() {
        let encodable_pkd_id = encode::encodable_package_id(pkg_id);
        if let Some(prev_pkg_id) = unique_pkg_ids.insert(encodable_pkd_id, pkg_id) {
            bail!(
                "package collision in the lockfile: packages {} and {} are different, \
                 but only one can be written to lockfile unambigiously",
                prev_pkg_id,
                pkg_id
            )
        }
    }
    Ok(())
}
