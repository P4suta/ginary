// SPDX-License-Identifier: MIT OR Apache-2.0
//! The transitive closure of an application's dependencies.
//!
//! `gleam export erlang-shipment` writes one directory per application it
//! compiled, and nothing else. The applications those depend on that live in
//! the OTP distribution — `kernel`, `stdlib`, `crypto`, `ssl` — are not in the
//! shipment, and the ones that are must not be taken from the host runtime.
//! [`app_dependency_closure`] walks both trees at once and answers the only
//! question assembly asks: which applications go into the artifact, and where
//! is each one read from.
//!
//! The walk is a worklist over `.app` files, seeded with the roots the caller
//! named, the extras the configuration added, and `kernel` and `stdlib`, which
//! are seeds unconditionally because a BEAM that cannot boot them is not a
//! runtime. Each name is resolved by looking in the shipment first and the OTP
//! library second, and the three edge kinds an `.app` file declares are treated
//! differently:
//!
//! - `applications` and `included_applications` are required. A name that
//!   resolves nowhere is [`ClosureError::AppNotFound`].
//! - `optional_applications` are followed when they resolve and recorded in
//!   [`AppSet::skipped_optional`] when they do not, whatever stopped them from
//!   resolving. Failing to resolve one is never an error; that is what makes it
//!   optional.
//!
//! Names come out of `.app` files, which ginary does not write, so a name is
//! checked before it is interpolated into a path: one that is empty or holds a
//! separator, a `..` or a NUL byte is [`ClosureError::InvalidAppName`] rather
//! than a lookup outside the two trees.
//!
//! The result is deterministic. The applications live in a [`BTreeMap`], every
//! candidate list is sorted before it is examined, and
//! [`ResolvedApp::requested_by`] is sorted and deduplicated, so permuting
//! `roots`, permuting `extra`, or reading the directories back in a different
//! order all produce the same [`AppSet`], byte for byte, down to the JSON.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::appfile::{self, AppFileError};

/// The two applications every closure is seeded with.
///
/// A shipment never lists them and a runtime cannot start without them, so
/// they are seeds whether or not anything asked for them.
pub const ALWAYS: [&str; 2] = ["kernel", "stdlib"];

/// Where an application's `ebin` directory was found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AppSource {
    /// `<shipment>/<name>/ebin`, written by `gleam export erlang-shipment`.
    Shipment,
    /// `<otp_lib>/<name>-<vsn>/ebin`, part of the OTP distribution.
    Otp {
        /// The version in the directory name, which is what tells the
        /// assembler which directory to copy.
        vsn: String,
    },
}

/// Why an application is in the closure at all.
///
/// Seeds are the entry points; everything else is [`SeedKind::None`] and
/// reached through at least one edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedKind {
    /// Named in `roots`: the application being packaged.
    Root,
    /// Named in `extra`: `extra_applications` or `otp_applications`.
    Extra,
    /// One of [`ALWAYS`].
    Always,
    /// Not a seed. Reached from one.
    None,
}

impl SeedKind {
    /// Whether this application entered the worklist as a seed.
    pub fn is_seed(self) -> bool {
        !matches!(self, Self::None)
    }

    /// The word [`explain`] prints in the origin column for a seed.
    pub fn label(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Extra => "extra",
            Self::Always => "always",
            Self::None => "",
        }
    }
}

/// One application in the closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResolvedApp {
    /// The application name, which is also the key in [`AppSet::apps`].
    pub name: String,
    /// The version from the `.app` file's `vsn` property.
    pub vsn: String,
    /// Which tree the application was read from.
    pub source: AppSource,
    /// The directory holding `<name>.app`.
    pub ebin: PathBuf,
    /// The sibling `priv` directory, when there is one and it is a directory.
    pub priv_dir: Option<PathBuf>,
    /// The applications that list this one, sorted and deduplicated.
    ///
    /// Immediate requesters only: this is one edge, not a path. A seed carries
    /// an empty vector even when other applications list it, because a seed is
    /// in the closure on its own account and [`AppSet::chain`] uses the empty
    /// vector as its stopping condition. An application never lists itself
    /// either, however often its own `.app` file names it.
    pub requested_by: Vec<String>,
    /// Why the application is in the closure.
    pub seed: SeedKind,
}

impl ResolvedApp {
    /// Whether the application came from the shipment.
    pub fn is_shipment(&self) -> bool {
        matches!(self.source, AppSource::Shipment)
    }

    /// Whether the application came from the OTP library.
    pub fn is_otp(&self) -> bool {
        matches!(self.source, AppSource::Otp { .. })
    }
}

/// Every application an artifact needs, and where each one lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AppSet {
    /// The applications, keyed by name, in name order.
    pub apps: BTreeMap<String, ResolvedApp>,
    /// Recoverable problems, in the order they were found. An application
    /// present in both trees is the canonical case: the shipment wins and the
    /// shadowing is recorded rather than silently accepted, as
    ///
    /// ```text
    /// `crypto` is in both trees; using the shipment copy at
    /// `<shipment>/crypto/ebin` and ignoring the OTP copy at
    /// `<otp_lib>/crypto-5.9.2/ebin`
    /// ```
    ///
    /// on one line. Every directory that lost is named, because the point of
    /// the warning is that the reader can tell what was dropped — including
    /// the case where the OTP library holds two versions of an application the
    /// shipment shadows, which is a warning rather than an
    /// [`ClosureError::AmbiguousOtpApp`] precisely because neither directory
    /// was going to be read.
    ///
    /// The other entries explain a skip: an `optional_applications` entry that
    /// is ambiguous in the OTP library, or that is not a usable name, is
    /// listed in [`AppSet::skipped_optional`] like any other, and says here
    /// why it could not resolve.
    pub warnings: Vec<String>,
    /// `optional_applications` that resolved nowhere, as
    /// `(application, requested_by)`, sorted and deduplicated. Not an error,
    /// but not silence either.
    pub skipped_optional: Vec<(String, String)>,
}

impl AppSet {
    /// The applications in name order.
    pub fn iter(&self) -> impl Iterator<Item = &ResolvedApp> {
        self.apps.values()
    }

    /// The application names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.apps.keys().cloned().collect()
    }

    /// One application by name.
    pub fn get(&self, name: &str) -> Option<&ResolvedApp> {
        self.apps.get(name)
    }

    /// How many applications the closure holds.
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    /// Whether the closure holds no applications.
    ///
    /// A default [`AppSet`] is empty. One that [`app_dependency_closure`]
    /// returned never is: `kernel` and `stdlib` are seeds unconditionally, so
    /// either they resolved or the call failed.
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// The applications read from the OTP library, in name order.
    pub fn otp_apps(&self) -> Vec<&ResolvedApp> {
        self.iter().filter(|app| app.is_otp()).collect()
    }

    /// The applications read from the shipment, in name order.
    pub fn shipment_apps(&self) -> Vec<&ResolvedApp> {
        self.iter().filter(|app| app.is_shipment()).collect()
    }

    /// One shortest path from a seed to `name`, ending in `name` itself.
    ///
    /// A breadth-first walk backwards over [`ResolvedApp::requested_by`], so
    /// the answer is the shortest explanation of why the application is in the
    /// closure rather than the first one the worklist happened to find. A seed
    /// answers with just its own name; a name that is not in the closure
    /// answers with an empty vector.
    pub fn chain(&self, name: &str) -> Vec<String> {
        if !self.apps.contains_key(name) {
            return Vec::new();
        }
        shortest_chain(name, |node| match self.apps.get(node) {
            Some(app) => app.requested_by.clone(),
            None => Vec::new(),
        })
    }
}

impl<'a> IntoIterator for &'a AppSet {
    type Item = &'a ResolvedApp;
    type IntoIter = std::collections::btree_map::Values<'a, String, ResolvedApp>;

    fn into_iter(self) -> Self::IntoIter {
        self.apps.values()
    }
}

/// Why a closure could not be computed.
#[derive(Debug)]
pub enum ClosureError {
    /// More than one directory under the OTP library matches `<name>-<vsn>`.
    ///
    /// Two versions of the same application in one library is a broken
    /// installation, and picking one would put an arbitrary choice into an
    /// artifact that is supposed to be reproducible.
    ///
    /// Only a *required* application the shipment does not hold gets this far.
    /// A shipment copy wins over the OTP library, and an optional dependency
    /// that does not resolve is never an error, so both of those record the
    /// candidates in [`AppSet::warnings`] and carry on: the ambiguity is
    /// reported where it decides nothing, and refused where it would.
    AmbiguousOtpApp {
        /// The application that matched more than once.
        name: String,
        /// The matching directory names, sorted.
        candidates: Vec<String>,
    },
    /// A required application is in neither tree.
    AppNotFound {
        /// The application that was not found.
        name: String,
        /// The full chain from a seed, ending in `name`. A missing seed
        /// answers with just its own name.
        requested_by: Vec<String>,
        /// The paths that were looked at, in the order they were tried.
        searched: Vec<PathBuf>,
    },
    /// A name that is not usable as a directory name reached the worklist.
    ///
    /// Every lookup interpolates the name into a path, so a name holding a
    /// separator, a `..` or a NUL byte would send the closure outside the two
    /// trees it is allowed to read — and an absolute one would leave them
    /// altogether, because [`Path::join`] with an absolute path discards the
    /// prefix. Names come out of `.app` files, which ginary does not write.
    InvalidAppName {
        /// The name that cannot be used.
        name: String,
        /// The full chain from a seed, ending in `name`. A seed answers with
        /// just its own name.
        requested_by: Vec<String>,
    },
    /// An `.app` file in one of the trees could not be read.
    AppFile {
        /// The file that could not be read.
        path: PathBuf,
        /// Why it could not be read.
        source: AppFileError,
    },
}

/// The advice [`ClosureError::AppNotFound`] ends with.
///
/// A missing application is nearly always a name OTP knows and the shipment
/// does not, so the fix is a line in `gleam.toml` rather than a bug report.
const NOT_FOUND_HINT: &str = concat!(
    "  hint: add it to `[erlang] extra_applications` (bundled and started) or\n",
    "        `[tools.ginary] otp_applications` (bundled only) in gleam.toml, or\n",
    "        check the dependency's .app file."
);

/// The rule [`ClosureError::InvalidAppName`] ends with.
const INVALID_NAME_RULE: &str = concat!(
    "  an application name is also a directory name: it cannot be empty and\n",
    "  cannot hold `/`, `\\`, `..` or a NUL byte."
);

/// Writes the `required by:` line the two name errors share.
fn write_requested_by(f: &mut fmt::Formatter<'_>, requested_by: &[String]) -> fmt::Result {
    if requested_by.len() > 1 {
        writeln!(f, "  required by: {}", requested_by.join(" -> "))
    } else {
        writeln!(f, "  required by: nothing; it was asked for directly")
    }
}

impl fmt::Display for ClosureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousOtpApp { name, candidates } => write!(
                f,
                "application `{name}` matches more than one directory in the OTP library: {}",
                candidates.join(", ")
            ),
            Self::AppNotFound {
                name,
                requested_by,
                searched,
            } => {
                writeln!(f, "application `{name}` was not found")?;
                write_requested_by(f, requested_by)?;
                for (index, path) in searched.iter().enumerate() {
                    let label = if index == 0 {
                        "  searched: "
                    } else {
                        "            "
                    };
                    writeln!(f, "{label}{}", path.display())?;
                }
                write!(f, "{NOT_FOUND_HINT}")
            }
            Self::InvalidAppName { name, requested_by } => {
                writeln!(f, "`{name}` is not a usable application name")?;
                write_requested_by(f, requested_by)?;
                write!(f, "{INVALID_NAME_RULE}")
            }
            Self::AppFile { path, source: _ } => {
                // The reason is the next link of the chain, not this one:
                // `src/main.rs` prints one line per link, and a layer that
                // repeats its own cause is printed twice.
                write!(f, "cannot read the application file `{}`", path.display())
            }
        }
    }
}

impl std::error::Error for ClosureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AppFile { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Resolves every application reachable from `roots` and `extra`.
///
/// `shipment` is the directory `gleam export erlang-shipment` wrote, one
/// subdirectory per application. `otp_lib` is the `lib` directory of an OTP
/// installation, one `<name>-<vsn>` subdirectory per application. The seeds
/// are `roots`, `extra` and [`ALWAYS`]; a name in more than one of those lists
/// keeps the first [`SeedKind`] in that order.
///
/// A name is resolved by looking for `<shipment>/<name>/ebin/<name>.app`
/// first and `<otp_lib>/<name>-<vsn>` second, where `<vsn>` must be digits
/// separated by dots and nothing else, so a `kernel-doc` or a
/// `kernel-11.0.3.bak` beside a real directory changes nothing.
///
/// # Errors
///
/// [`ClosureError::AppNotFound`] when a required application is in neither
/// tree; [`ClosureError::InvalidAppName`] when a required name cannot be used
/// as a directory name, before any path is built from it;
/// [`ClosureError::AmbiguousOtpApp`] when a required application has to come
/// from the OTP library and the library holds more than one version of it —
/// a shipment copy ends the lookup, so an application the shipment provides
/// is a warning rather than an error however many OTP versions sit beside it;
/// and [`ClosureError::AppFile`] when an `.app` file cannot be read.
///
/// Only a *required* application reaches any of these. An optional dependency
/// that fails to resolve, for any of those reasons, is recorded in
/// [`AppSet::skipped_optional`] and [`AppSet::warnings`] instead.
pub fn app_dependency_closure(
    shipment: &Path,
    otp_lib: &Path,
    roots: &[String],
    extra: &[String],
) -> Result<AppSet, ClosureError> {
    let library = OtpLibrary::index(otp_lib);
    let seeds = seed_kinds(roots, extra);

    // The worklist is ordered by name rather than by discovery, so the order
    // `roots` and `extra` were given in cannot reach the output: not through
    // `warnings`, not through which of two errors is reported first, and not
    // through the chain a missing application names.
    let mut pending: BTreeSet<String> = seeds.keys().cloned().collect();
    let mut requesters: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut apps: BTreeMap<String, ResolvedApp> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut skipped_optional: BTreeSet<(String, String)> = BTreeSet::new();

    // A name leaves `pending` exactly once and never returns, because it is in
    // `apps` from then on. That is what makes a cycle and a self-reference
    // ordinary rather than special cases.
    while let Some(name) = pending.pop_first() {
        if apps.contains_key(&name) {
            continue;
        }

        let found = match locate(shipment, &library, &name) {
            Resolution::Found(found) => found,
            Resolution::Missing => {
                return Err(ClosureError::AppNotFound {
                    requested_by: shortest_chain(&name, |node| {
                        upstream_of(node, &seeds, &requesters)
                    }),
                    searched: searched_paths(shipment, &library, &name),
                    name,
                });
            }
            Resolution::Ambiguous { candidates } => {
                return Err(ClosureError::AmbiguousOtpApp { name, candidates });
            }
            Resolution::Unusable => {
                return Err(ClosureError::InvalidAppName {
                    requested_by: shortest_chain(&name, |node| {
                        upstream_of(node, &seeds, &requesters)
                    }),
                    name,
                });
            }
        };
        if let Some(warning) = found.shadow_warning(&name) {
            warnings.push(warning);
        }

        let app_file = found.ebin.join(format!("{name}.app"));
        let resource =
            appfile::parse_app_file(&app_file).map_err(|source| ClosureError::AppFile {
                path: app_file,
                source,
            })?;

        // `optional_applications` is a subset of `applications` by OTP's own
        // rule, so the required edges are the difference: a name in both lists
        // is optional, and reading `applications` alone would turn every
        // optional dependency into a hard one.
        let optional: BTreeSet<&String> = resource.optional_applications.iter().collect();
        for dep in resource
            .applications
            .iter()
            .chain(resource.included_applications.iter())
        {
            if dep == &name || optional.contains(dep) {
                continue;
            }
            requesters
                .entry(dep.clone())
                .or_default()
                .insert(name.clone());
            if !apps.contains_key(dep) {
                pending.insert(dep.clone());
            }
        }
        // An optional edge is probed, not required: a name that does not
        // resolve is recorded rather than refused. That covers all three ways
        // it can fail to resolve, because a library holding two versions and a
        // name that is not a directory name are as unusable as an absent
        // application — and each of the two says so in `warnings`, since a
        // skip is a reported decision.
        for dep in &resource.optional_applications {
            if dep == &name {
                continue;
            }
            if apps.contains_key(dep) {
                requesters
                    .entry(dep.clone())
                    .or_default()
                    .insert(name.clone());
                continue;
            }
            match locate(shipment, &library, dep) {
                Resolution::Found(_) => {
                    requesters
                        .entry(dep.clone())
                        .or_default()
                        .insert(name.clone());
                    pending.insert(dep.clone());
                }
                unresolved => {
                    if let Some(reason) = unresolved.skip_reason() {
                        warnings.push(format!(
                            "optional application `{dep}`, requested by `{name}`, \
                             was skipped: {reason}"
                        ));
                    }
                    skipped_optional.insert((dep.clone(), name.clone()));
                }
            }
        }

        let priv_dir = found.root.join("priv");
        apps.insert(
            name.clone(),
            ResolvedApp {
                vsn: resource.vsn,
                source: found.source,
                ebin: found.ebin,
                priv_dir: priv_dir.is_dir().then_some(priv_dir),
                requested_by: Vec::new(),
                seed: seeds.get(&name).copied().unwrap_or(SeedKind::None),
                name,
            },
        );
    }

    // `requested_by` is filled in only now, because an application can be
    // resolved long before the last edge into it is discovered. A seed keeps
    // the empty vector it was built with: it is in the closure on its own
    // account, and `chain` uses that emptiness as its stopping condition.
    for (name, app) in &mut apps {
        if app.seed.is_seed() {
            continue;
        }
        if let Some(found) = requesters.get(name) {
            app.requested_by = found.iter().cloned().collect();
        }
    }

    Ok(AppSet {
        apps,
        warnings,
        skipped_optional: skipped_optional.into_iter().collect(),
    })
}

/// The seed kind of every name in `roots`, `extra` and [`ALWAYS`].
///
/// A name in more than one list keeps the first kind in that order, so
/// `--root kernel` is a [`SeedKind::Root`] and not a [`SeedKind::Always`].
fn seed_kinds(roots: &[String], extra: &[String]) -> BTreeMap<String, SeedKind> {
    let mut seeds: BTreeMap<String, SeedKind> = BTreeMap::new();
    for (names, kind) in [(roots, SeedKind::Root), (extra, SeedKind::Extra)] {
        for name in names {
            seeds.entry(name.clone()).or_insert(kind);
        }
    }
    for name in ALWAYS {
        seeds.entry(name.to_owned()).or_insert(SeedKind::Always);
    }
    seeds
}

/// The immediate requesters of `node` during the walk, empty for a seed.
///
/// Seeds answer empty even when something lists them, which is what stops the
/// backwards walk [`ClosureError::AppNotFound`] uses to build its chain.
fn upstream_of(
    node: &str,
    seeds: &BTreeMap<String, SeedKind>,
    requesters: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    if seeds.contains_key(node) {
        return Vec::new();
    }
    match requesters.get(node) {
        Some(found) => found.iter().cloned().collect(),
        None => Vec::new(),
    }
}

/// One shortest path from a seed to `name`, ending in `name`.
///
/// `upstream` answers the immediate requesters of a name, sorted, and answers
/// with an empty vector for a seed. The walk is breadth first from `name`
/// backwards, so the first seed it reaches is a nearest one and the path it
/// returns is a shortest one. A name no seed reaches answers empty.
fn shortest_chain<F>(name: &str, upstream: F) -> Vec<String>
where
    F: Fn(&str) -> Vec<String>,
{
    let mut queue: VecDeque<String> = VecDeque::from([name.to_owned()]);
    let mut seen: BTreeSet<String> = BTreeSet::from([name.to_owned()]);
    // `towards[a] == b` means `b` is one step closer to `name` than `a` is.
    let mut towards: BTreeMap<String, String> = BTreeMap::new();

    while let Some(current) = queue.pop_front() {
        let up = upstream(&current);
        if up.is_empty() {
            let mut chain = vec![current.clone()];
            let mut node = current;
            while let Some(next) = towards.get(&node) {
                chain.push(next.clone());
                node = next.clone();
            }
            return chain;
        }
        for requester in up {
            if seen.insert(requester.clone()) {
                towards.insert(requester.clone(), current.clone());
                queue.push_back(requester);
            }
        }
    }
    Vec::new()
}

/// The two paths a name is looked for at, in the order they are tried.
///
/// The OTP path carries the literal `<vsn>` because the version is part of what
/// was searched for: naming one version would suggest that directory exists and
/// could not be read, which is a different problem.
fn searched_paths(shipment: &Path, library: &OtpLibrary, name: &str) -> Vec<PathBuf> {
    vec![
        shipment.join(name).join("ebin").join(format!("{name}.app")),
        library.lib.join(format!("{name}-<vsn>")),
    ]
}

/// Where one application was found, before its `.app` file is read.
struct Found {
    /// The application's directory, the parent of `ebin` and of `priv`.
    root: PathBuf,
    /// The directory holding `<name>.app`.
    ebin: PathBuf,
    /// Which tree `ebin` is in.
    source: AppSource,
    /// The OTP `ebin` directories that lost to the shipment copy, sorted.
    ///
    /// Empty unless the application came from the shipment. It holds more than
    /// one entry when the OTP library is broken *and* the shipment shadows it,
    /// which is a warning rather than an error: the directories the closure
    /// declined to choose between are ones it was never going to read.
    shadowed_otp: Vec<PathBuf>,
}

impl Found {
    /// The warning a shipment copy that shadows the OTP library produces.
    fn shadow_warning(&self, name: &str) -> Option<String> {
        let shipment = self.ebin.display();
        match self.shadowed_otp.as_slice() {
            [] => None,
            [otp] => Some(format!(
                "`{name}` is in both trees; using the shipment copy at `{shipment}` \
                 and ignoring the OTP copy at `{}`",
                otp.display()
            )),
            many => Some(format!(
                "`{name}` is in both trees; using the shipment copy at `{shipment}` \
                 and ignoring the OTP library, which holds more than one version \
                 of it: {}",
                many.iter()
                    .map(|path| format!("`{}`", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

/// What looking one name up in both trees answered.
enum Resolution {
    /// The application was found, in the shipment or in the OTP library.
    Found(Found),
    /// Neither tree holds it.
    Missing,
    /// The shipment does not hold it and the OTP library holds more than one
    /// version, so there is no single directory to read it from.
    Ambiguous {
        /// The matching directory names, sorted.
        candidates: Vec<String>,
    },
    /// The name cannot be turned into a directory name, so nothing was looked
    /// at: no path was built from it and no directory was stat'ed.
    Unusable,
}

impl Resolution {
    /// Why an optional dependency was skipped, for a resolution that is not a
    /// [`Resolution::Found`]. An absent application needs no explanation: it
    /// is already reported by name in [`AppSet::skipped_optional`].
    fn skip_reason(&self) -> Option<String> {
        match self {
            Self::Found(_) | Self::Missing => None,
            Self::Ambiguous { candidates } => Some(format!(
                "it matches more than one directory in the OTP library: {}",
                candidates.join(", ")
            )),
            Self::Unusable => Some("it is not a usable application name".to_owned()),
        }
    }
}

/// Looks one name up in the shipment and then in the OTP library.
///
/// The order is the documented one and it decides more than which copy wins.
/// The shipment is probed first, and a shipment hit ends the lookup: the OTP
/// candidates are then only the material of a warning, so a library holding
/// two versions of an application the shipment shadows cannot fail a build
/// that would never have read either directory. The OTP side is ambiguous
/// only when the answer actually has to come from there.
fn locate(shipment: &Path, library: &OtpLibrary, name: &str) -> Resolution {
    if !is_usable_app_name(name) {
        return Resolution::Unusable;
    }
    let candidates = library.matches(name);
    let shipment_root = shipment.join(name);
    let shipment_ebin = shipment_root.join("ebin");

    if shipment_ebin.join(format!("{name}.app")).is_file() {
        return Resolution::Found(Found {
            root: shipment_root,
            ebin: shipment_ebin,
            source: AppSource::Shipment,
            shadowed_otp: candidates
                .iter()
                .map(|(dir, _)| library.lib.join(dir).join("ebin"))
                .collect(),
        });
    }
    match candidates {
        [] => Resolution::Missing,
        [(dir, vsn)] => {
            let root = library.lib.join(dir);
            Resolution::Found(Found {
                ebin: root.join("ebin"),
                root,
                source: AppSource::Otp { vsn: vsn.clone() },
                shadowed_otp: Vec::new(),
            })
        }
        many => Resolution::Ambiguous {
            candidates: many.iter().map(|(dir, _)| dir.clone()).collect(),
        },
    }
}

/// Whether `name` can be used as the directory name every lookup builds.
///
/// Names come out of `.app` files, which ginary does not write, and every
/// lookup interpolates one into a path. A name holding a separator or a `..`
/// would reach outside the two trees, an absolute one would leave them
/// entirely — [`Path::join`] with an absolute path discards the prefix — and an
/// empty one would name the tree itself. None of those is an application name;
/// the rule is documented in [`INVALID_NAME_RULE`].
fn is_usable_app_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', '\0']) && !name.contains("..")
}

/// The `<name>-<vsn>` directories of an OTP `lib`, indexed by name.
///
/// The directory is listed once, at the start, so that the answer cannot depend
/// on when a name happens to be asked for, and so that a library holding three
/// hundred applications is not read three hundred times.
struct OtpLibrary {
    /// The `lib` directory itself.
    lib: PathBuf,
    /// Application name to its `(directory, version)` candidates, sorted.
    candidates: BTreeMap<String, Vec<(String, String)>>,
}

impl OtpLibrary {
    /// Indexes `lib`, ignoring everything that is not a versioned directory.
    ///
    /// A `lib` that cannot be read is indexed as empty rather than reported
    /// here: the first application that needed it fails with
    /// [`ClosureError::AppNotFound`], whose `searched` list names the path.
    fn index(lib: &Path) -> Self {
        let mut candidates: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(lib) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let Some(dir) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Some((name, vsn)) = split_versioned(&dir) else {
                    continue;
                };
                candidates.entry(name).or_default().push((dir, vsn));
            }
        }
        for found in candidates.values_mut() {
            found.sort();
        }
        Self {
            lib: lib.to_path_buf(),
            candidates,
        }
    }

    /// Every `(directory, version)` the library holds for `name`, sorted.
    ///
    /// Empty, one entry, or an ambiguity: which of the three is a problem is
    /// [`locate`]'s decision, because it depends on whether the shipment
    /// already answered and on whether the edge is a required one.
    fn matches(&self, name: &str) -> &[(String, String)] {
        match self.candidates.get(name) {
            Some(found) => found.as_slice(),
            None => &[],
        }
    }
}

/// Splits `<name>-<vsn>` at the last `-`, if the tail is a version.
///
/// The tail must be digits separated by single dots and nothing else, so
/// `crypto-doc`, `crypto-latest`, `crypto-5.9.2.bak` and `crypto-` are not
/// versioned directories, while `odbc-3` and `my-app-1.0` are.
fn split_versioned(dir: &str) -> Option<(String, String)> {
    let (name, vsn) = dir.rsplit_once('-')?;
    if name.is_empty() || !is_version(vsn) {
        return None;
    }
    Some((name.to_owned(), vsn.to_owned()))
}

/// Whether `text` matches `^[0-9]+(\.[0-9]+)*$`.
fn is_version(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Renders an [`AppSet`] as a table of name, version, source and origin.
///
/// The origin column answers "why is this here": `root`, `extra` or `always`
/// for a seed, and the chain from a seed for everything else, which is what
/// `ginary build --explain` prints when an artifact is larger than expected.
/// The table ends in a newline; an empty set renders as the header alone.
pub fn explain(set: &AppSet) -> String {
    let rows: Vec<[String; 4]> = set
        .iter()
        .map(|app| {
            let origin = if app.seed.is_seed() {
                app.seed.label().to_owned()
            } else {
                set.chain(&app.name).join(" -> ")
            };
            [
                app.name.clone(),
                app.vsn.clone(),
                source_label(&app.source).to_owned(),
                origin,
            ]
        })
        .collect();
    render_table(["name", "vsn", "source", "origin"], &rows)
}

/// The word the `source` column prints for one application.
pub(crate) fn source_label(source: &AppSource) -> &'static str {
    match source {
        AppSource::Shipment => "shipment",
        AppSource::Otp { .. } => "otp",
    }
}

/// Renders a table with a header row and columns padded to their widest cell.
///
/// The last column is never padded, so no line carries trailing spaces.
pub(crate) fn render_table<const N: usize>(header: [&str; N], rows: &[[String; N]]) -> String {
    let mut widths = header.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }

    let mut text = String::new();
    let mut push = |cells: [&str; N]| {
        for (index, cell) in cells.iter().enumerate() {
            if index + 1 == N {
                text.push_str(cell);
            } else {
                text.push_str(&format!("{cell:width$}  ", width = widths[index]));
            }
        }
        text.push('\n');
    };

    push(header);
    for row in rows {
        push(std::array::from_fn(|index| row[index].as_str()));
    }
    text
}
