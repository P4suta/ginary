// SPDX-License-Identifier: MIT OR Apache-2.0
//! The two namespaces `otp_variant` lives in disagreed, so one of the
//! catalogue's own variant names could not be written in a `gleam.toml`.
//!
//! `config::OTP_VARIANTS` was `["static", "dynamic"]` and `validate_targets`
//! refused anything else. The catalogue's own default variant name is
//! `default` — `catalog::DEFAULT_VARIANT`, the key `upstream_asset` maps to
//! `-glibc`, and the key `dist/otp/catalog.json` files the gnu runtime under.
//! C3 is the milestone that gave `otp_variant` meaning: `bundle` passes it
//! straight into `Catalog::select`. So `CatalogError::AmbiguousVariant` could
//! say
//!
//! ```text
//! OTP 29.0.5 for linux-x86_64-gnu has default, static and no default;
//! name one with `otp_variant`
//! ```
//!
//! and the value it named would then be refused by the manifest reader.
//!
//! The right behaviour: the set the configuration accepts is the catalogue's
//! variant namespace, so every name the pipeline can produce is a name a user
//! can write down.
#![cfg(feature = "cli")]

use std::path::Path;

use ginary::config::{self, ConfigError, OTP_VARIANTS, ProjectConfig};

/// The manifest path every reading here names.
const MANIFEST: &str = "/w/gleam.toml";

/// A one-target manifest naming `variant`.
fn manifest(target: &str, variant: &str) -> String {
    format!(
        "name = \"app\"\nversion = \"1.0.0\"\n\n\
         [tools.ginary.target.\"{target}\"]\notp_variant = \"{variant}\"\n"
    )
}

#[test]
fn every_variant_the_repack_pipeline_can_produce_is_one_a_manifest_may_name() {
    // The pipeline's own table is the authority on what names exist: whatever
    // `upstream_asset` maps to an asset is a runtime the catalogue can hold,
    // and a runtime the catalogue can hold is one a target may ask for.
    let combinations = [
        ("linux-x86_64-musl", "static"),
        ("linux-x86_64-musl", "dynamic"),
        ("linux-aarch64-musl", "static"),
        ("linux-aarch64-musl", "dynamic"),
        ("linux-x86_64-gnu", ginary::catalog::DEFAULT_VARIANT),
        ("linux-aarch64-gnu", ginary::catalog::DEFAULT_VARIANT),
    ];

    for (target, variant) in combinations {
        ginary::catalog::upstream_asset("29.0.5", target, variant).unwrap_or_else(|error| {
            panic!(
                "the fixture combination {target}:{variant} must be one upstream builds: {error}"
            )
        });
        assert!(
            OTP_VARIANTS.contains(&variant),
            "`{variant}` is a name the catalogue uses and the manifest reader refuses"
        );
        let config = ProjectConfig::from_toml(&manifest(target, variant), Path::new(MANIFEST));
        assert!(
            config.is_ok(),
            "[tools.ginary.target.{target}] otp_variant = \"{variant}\" must parse: {:?}",
            config.err()
        );
    }
}

#[test]
fn a_variant_no_catalog_could_name_is_still_refused() {
    let error = ProjectConfig::from_toml(
        &manifest("linux-x86_64-musl", "hybrid"),
        Path::new(MANIFEST),
    )
    .expect_err("`hybrid` is not a variant anything builds");

    assert!(
        matches!(&error, ConfigError::OtpVariant { value, .. } if value == "hybrid"),
        "expected ConfigError::OtpVariant, got {error:?}"
    );
    let sentence = error.to_string();
    for name in OTP_VARIANTS {
        assert!(
            sentence.contains(name),
            "the message lists the names that would work: {sentence}"
        );
    }
    let _ = config::OTP_VARIANTS;
}
