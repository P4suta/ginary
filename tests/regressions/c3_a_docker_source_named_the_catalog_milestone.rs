// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `docker:` runtime was refused naming the wrong milestone, and the
//! sentence around it listed sources that had stopped being the whole list.
//!
//! C3 implemented `catalog` and `tarball:PATH`, and left both refusal sites
//! spelling `CATALOG_MILESTONE` for *every* source that did not resolve there.
//! So `erts = "docker:erlang:29-alpine"` failed a build with
//!
//! ```text
//! the ERTS source `docker:erlang:29-alpine` arrives with the catalog
//! milestone; only `host` and `dir:PATH` are available today
//! ```
//!
//! while `ginary doctor`, which asks `ErtsSourceSpec::milestone`, printed
//! "arrives with the container image milestone" for the same value — two
//! answers to one question, one of them naming a milestone that had already
//! shipped, and a trailing clause that was false the moment `catalog` worked.
//!
//! The right behaviour: one answer. The milestone a build names is the
//! milestone `milestone()` reports, the sentence does not claim a source is
//! unavailable when it is, and the two sources that *are* here but need a
//! cache root and a catalogue earn a refusal of their own that says so rather
//! than borrowing a milestone.
#![cfg(feature = "cli")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginary::catalog::{CatalogPaths, OtpReq};
use ginary::config::TargetConfig;
use ginary::diag::Diag;
use ginary::download::Net;
use ginary::erts_source::{self, ErtsError, ErtsSourceSpec, IMAGE_MILESTONE, SourceContext};
use ginary::target::Target;

/// The image spelling every assertion here is about.
const IMAGE: &str = "docker:erlang:29-alpine";

/// A context whose paths are never read, because every source that reaches it
/// here is refused before anything is opened.
fn context<'a>(
    paths: &'a CatalogPaths,
    cache: &'a Path,
    net: &'a Net,
    req: &'a OtpReq,
    diag: &'a Diag,
) -> SourceContext<'a> {
    SourceContext {
        catalog_paths: paths,
        cache_root: cache,
        net,
        host_release: 29,
        otp_version: req,
        variant: None,
        diag,
    }
}

#[test]
fn a_docker_source_is_refused_with_the_milestone_the_spec_itself_reports() {
    let spec: ErtsSourceSpec = IMAGE.parse().expect("`docker:IMAGE` parses");
    let reported = spec.milestone().expect("a container image is not here yet");

    let paths = CatalogPaths::default();
    let cache = PathBuf::from("/nonexistent/ginary-cache");
    let net = Net::offline();
    let req = OtpReq::Host(29);
    let diag = Diag::disabled();
    let ctx = context(&paths, &cache, &net, &req, &diag);

    // The arm a build actually takes: `bundle` calls `resolve_in` whenever any
    // target names a source that needs a cache, which a `docker:` project does
    // as soon as one other target names `catalog`.
    let error = erts_source::resolve_in(&spec, &Target::host(), &ctx)
        .expect_err("a container image is not a runtime ginary can read yet");

    match &error {
        ErtsError::NotYetAvailable {
            spec: written,
            milestone,
        } => {
            assert_eq!(written, IMAGE);
            assert_eq!(
                *milestone, reported,
                "the milestone a build names is the milestone `milestone()` reports"
            );
            assert_eq!(*milestone, IMAGE_MILESTONE);
        }
        other => panic!("expected NotYetAvailable, got {other:?}"),
    }

    let sentence = error.to_string();
    assert!(
        sentence.contains("arrives with the container image milestone"),
        "the sentence names the milestone it is really waiting on: {sentence}"
    );
    assert!(
        !sentence.contains("only `host` and `dir:PATH` are available today"),
        "and does not claim the two sources C3 shipped are still to come: {sentence}"
    );
}

#[test]
fn the_context_free_entry_point_refuses_a_docker_source_the_same_way() {
    let spec: ErtsSourceSpec = IMAGE.parse().expect("`docker:IMAGE` parses");
    let error = erts_source::resolve(&spec, &Target::host())
        .expect_err("a container image is not a runtime ginary can read yet");

    assert!(
        matches!(
            &error,
            ErtsError::NotYetAvailable { milestone, .. } if *milestone == IMAGE_MILESTONE
        ),
        "one milestone, whichever entry point asked: {error:?}"
    );
}

#[test]
fn a_catalog_or_a_tarball_without_a_context_says_it_needs_one_rather_than_a_milestone() {
    for spec in [
        ErtsSourceSpec::Catalog,
        ErtsSourceSpec::Tarball(PathBuf::from("/tmp/otp.tar.zst")),
    ] {
        let error = erts_source::resolve(&spec, &Target::host())
            .expect_err("neither resolves without a cache root and a catalogue");
        match &error {
            ErtsError::NeedsContext { spec: written } => assert_eq!(*written, spec.label()),
            other => panic!("expected NeedsContext for {}, got {other:?}", spec.label()),
        }
        let sentence = error.to_string();
        assert!(
            !sentence.contains("milestone"),
            "a source that is here does not arrive with a milestone: {sentence}"
        );
    }
}

#[test]
fn doctor_and_a_build_give_one_answer_for_one_docker_source() {
    let host = Target::host();
    let config = BTreeMap::from([(
        host.name(),
        TargetConfig {
            erts: Some(IMAGE.to_owned()),
            ..TargetConfig::default()
        },
    )]);

    let rows = ginary::doctor::probe_targets(&[host], &config);
    let row = rows.first().expect("one target, one row");
    let detail = row.detail.clone().unwrap_or_default();

    let spec: ErtsSourceSpec = IMAGE.parse().expect("`docker:IMAGE` parses");
    let error = erts_source::resolve(&spec, &host).expect_err("a build refuses it");

    assert!(
        detail.contains(IMAGE_MILESTONE),
        "`doctor` names the milestone: {detail}"
    );
    assert!(
        error.to_string().contains(IMAGE_MILESTONE),
        "and so does the build that refuses the same value: {error}"
    );
}
