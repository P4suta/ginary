// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release workflow asked `ginary otp repack` for macOS, which it does not
//! do.
//!
//! **What went wrong.** `distribute.yml`'s `build-macos` job runs
//! `ginary otp repack --targets "${{ matrix.target }}"` for `macos-x86_64` and
//! `macos-aarch64`. `catalog::upstream_asset` — the one function the repack
//! pipeline maps a target through — matches `linux-*` and returns
//! `RepackError::NoUpstreamAsset` for everything else:
//!
//! ```console
//! $ ginary otp repack --upstream-tag OTP-29.0.5 --targets macos-aarch64
//! error: gleam-community/erlang-linux-builds has no asset for macos-aarch64:default
//! ```
//!
//! So both macOS jobs of the release build would have died on that step, on
//! the first release anybody cut. `distribute.yml` has never run — it waits on
//! a tag — so nothing had reported it.
//!
//! The tree already said macOS repack was not implemented. `README.md` records
//! it as "scoped out of this pass" and `docs/dev/log/D3.md` says why: the trust
//! anchor reads a repackaged `beam.smp` with `macho.rs`, which needs
//! `repack_one` generalised over object format and a Mach-O-aware strip,
//! neither of which exists. `catalog::erlef_upstream_asset` names the asset
//! `erlef/otp_builds` publishes and is called by nothing — it is a pinned fact,
//! not a code path, and reading it as support for macOS repacking is exactly
//! the mistake this file exists to stop.
//!
//! **The input.** Any workflow step that asks `otp repack` for a target the
//! asset mapping cannot name.
//!
//! **The correct behaviour.** Every target a workflow hands `--targets` has to
//! be one `catalog::upstream_asset` can map. Derived, not listed: the rule
//! calls the same function the pipeline does, so a target that becomes
//! supported needs no edit here and one that stops being supported fails.

use ginary::catalog::upstream_asset;
use saphyr::YamlOwned;

use crate::common::repo::{shell_code, yaml, yaml_files_under};

/// The version and variant the rule probes the mapping with.
///
/// The mapping is a function of target, variant and version, and only the
/// target is what a workflow chooses; the other two are held at what
/// `distribute.yml` itself uses so that a refusal is about the target.
const PROBE_VERSION: &str = "29.0.5";

/// The variants a target may be repacked as, tried in turn.
///
/// A target is repackable when *some* variant of it maps: `--targets
/// linux-x86_64-musl` takes the default variant, and `linux-x86_64-musl:static`
/// names one. A rule that probed only one variant would call a musl target
/// unsupported.
const PROBE_VARIANTS: [&str; 3] = ["default", "static", "dynamic"];

/// Whether `ginary otp repack` can name an upstream asset for `target`.
fn repackable(target: &str) -> bool {
    PROBE_VARIANTS
        .iter()
        .any(|variant| upstream_asset(PROBE_VERSION, target, variant).is_ok())
}

/// One command split into words, with a quoted run kept together.
///
/// `--targets "${{ matrix.target }}"` is two words, not four. Splitting on
/// whitespace read the third of them as `${{`, which no mapping names — a rule
/// that reports a defect for a reason that is not the defect, and goes on
/// reporting it after the real one is fixed.
fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for character in command.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => word.push(character),
            None if character == '"' || character == '\'' => {
                quote = Some(character);
                started = true;
            }
            None if character.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(character);
                started = true;
            }
        }
    }
    if started {
        out.push(word);
    }
    out
}

/// The variables one script assigns, as `name` to value.
///
/// `distribute.yml`'s Linux job builds its selector in a `case`:
///
/// ```sh
/// case "${{ matrix.target }}" in
///   *-musl) spec="${{ matrix.target }}:static" ;;
///   *)      spec="${{ matrix.target }}" ;;
/// esac
/// ```
///
/// so the word `--targets` is handed is `$spec`, and a rule that stopped there
/// would report a shell variable as an unmappable target. Every assignment is
/// collected rather than the last, because a `case` is exactly the shape where
/// several of them are alternatives and each one has to hold.
fn assignments(script: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for command in script.replace("\\\n", " ").lines().map(shell_code) {
        for word in words(command) {
            let Some((name, value)) = word.split_once('=') else {
                continue;
            };
            let mut characters = name.chars();
            let Some(first) = characters.next() else {
                continue;
            };
            if (first.is_ascii_alphabetic() || first == '_')
                && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !value.is_empty()
            {
                out.push((name.to_owned(), value.to_owned()));
            }
        }
    }
    out
}

/// `value` with a leading `$name` or `${name}` resolved against `script`.
///
/// A variable with no assignment resolves to nothing rather than to itself: a
/// target this rule cannot see is one it must not accuse.
fn resolve(script: &str, value: &str) -> Vec<String> {
    // A GitHub expression opens with the same `$` a shell variable does and is
    // not one. Reading `${{ matrix.target }}` as a variable named
    // `matrix.target` finds no assignment and drops the target silently, which
    // is worse than the whitespace bug it replaced: the rule went green over a
    // workflow it was no longer reading.
    if value.trim_start().starts_with("${{") {
        return vec![value.to_owned()];
    }
    let Some(name) = value
        .strip_prefix('$')
        .map(|rest| rest.trim_matches(['{', '}']))
    else {
        return vec![value.to_owned()];
    };
    assignments(script)
        .into_iter()
        .filter(|(assigned, _)| assigned == name)
        .map(|(_, value)| value)
        .collect()
}

/// The `--targets` values one `run:` script passes to `otp repack`.
///
/// Comma separated, and a `<target>:<variant>` selector contributes its target
/// half. A value written as a shell variable is resolved against the script's
/// own assignments, and one written as a `${{ matrix.<key> }}` expression is
/// returned as written for [`expand`] to resolve against its job.
fn repack_targets(script: &str) -> Vec<String> {
    let joined = script.replace("\\\n", " ");
    let mut out = Vec::new();
    for command in joined.lines().map(shell_code) {
        if !command.contains("otp repack") {
            continue;
        }
        let command_words = words(command);
        let mut index = 0;
        while index < command_words.len() {
            let word = &command_words[index];
            let raw = match word.strip_prefix("--targets=") {
                Some(inline) => inline.to_owned(),
                None if word == "--targets" => {
                    index += 1;
                    command_words.get(index).cloned().unwrap_or_default()
                }
                None => {
                    index += 1;
                    continue;
                }
            };
            for value in resolve(script, &raw) {
                for selector in value.split(',') {
                    let target = selector.split(':').next().unwrap_or_default().trim();
                    if !target.is_empty() {
                        out.push(target.to_owned());
                    }
                }
            }
            index += 1;
        }
    }
    out
}

/// Every value a job's matrix gives `${{ matrix.<key> }}`.
///
/// `distribute.yml` writes `--targets "${{ matrix.target }}"`, so a rule that
/// read the literal would be checking the string `${{ matrix.target }}` — which
/// no mapping names, and which would fail this rule for the wrong reason and
/// keep failing after the defect was fixed.
fn expand(job: &YamlOwned, value: &str) -> Vec<String> {
    let Some(key) = value
        .trim()
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .map(str::trim)
        .and_then(|inner| inner.strip_prefix("matrix."))
    else {
        return vec![value.to_owned()];
    };
    let key = key.trim();
    let Some(include) = job
        .as_mapping_get("strategy")
        .and_then(|strategy| strategy.as_mapping_get("matrix"))
        .and_then(|matrix| matrix.as_mapping_get("include"))
        .and_then(YamlOwned::as_vec)
    else {
        return Vec::new();
    };
    include
        .iter()
        .filter_map(|row| row.as_mapping_get(key))
        .filter_map(|value| value.as_str())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_mapping_is_the_one_the_pipeline_uses() {
    // The rule's whole worth is that it asks the same function `repack_one`
    // asks. Both halves are asserted so that a mapping which grew or lost a
    // target is visible here rather than only in a release.
    assert!(
        repackable("linux-x86_64-gnu"),
        "the Linux targets are what upstream publishes and what the catalog is built from"
    );
    assert!(
        repackable("linux-aarch64-musl"),
        "a musl target maps through its `static` variant, which is the one upstream ships \
         without a suffix"
    );
    assert!(
        !repackable("macos-aarch64"),
        "`catalog::erlef_upstream_asset` names the asset erlef publishes and nothing calls it; \
         macOS repacking is recorded in docs/dev/log/D3.md as scoped out, and a rule that \
         thought otherwise would pass a release workflow that cannot run"
    );
    assert!(
        !repackable("windows-x86_64"),
        "a Windows runtime ships as otp_win64_<version>.zip, a shape the tarball pipeline does \
         not read"
    );
}

#[test]
fn the_scanner_reads_a_targets_argument_in_every_spelling() {
    assert_eq!(
        repack_targets("cargo run -- otp repack --targets \"macos-x86_64\" --out dist\n"),
        vec!["macos-x86_64".to_owned()],
        "a quoted single target"
    );
    assert_eq!(
        repack_targets(
            "cargo run --quiet -- otp repack \\\n  --upstream-tag OTP-29.0.5 \\\n  \
             --targets linux-x86_64-musl:static,linux-x86_64-gnu --out dist/otp\n"
        ),
        vec![
            "linux-x86_64-musl".to_owned(),
            "linux-x86_64-gnu".to_owned()
        ],
        "a wrapped command, a comma-separated list, and a `<target>:<variant>` selector whose \
         target half is what the mapping is asked about"
    );
    assert_eq!(
        repack_targets("cargo run -- otp repack --targets=macos-aarch64\n"),
        vec!["macos-aarch64".to_owned()],
        "the `--targets=` spelling is the same argument"
    );
    assert!(
        repack_targets("# cargo run -- otp repack --targets macos-aarch64\n").is_empty(),
        "a commented-out command asks for nothing; `shell_code` is the one stripper this \
         repository has"
    );
    // The two shapes `distribute.yml` actually writes, and the two that
    // defeated the first version of this scanner.
    assert_eq!(
        repack_targets("cargo run -- otp repack --targets \"${{ matrix.target }}\" --out d\n"),
        vec!["${{ matrix.target }}".to_owned()],
        "a quoted expression is one word; splitting on whitespace read it as `${{`"
    );
    assert_eq!(
        repack_targets(
            "case \"${{ matrix.target }}\" in\n               *-musl) spec=\"${{ matrix.target }}:static\" ;;\n               *)      spec=\"${{ matrix.target }}\" ;;\n             esac\n             cargo run -- otp repack --targets \"$spec\" --out d\n"
        ),
        vec![
            "${{ matrix.target }}".to_owned(),
            "${{ matrix.target }}".to_owned()
        ],
        "a selector built in a `case` resolves to every branch's value, each of which has to \
         map — `:static` is a variant and the target half is what is asked about"
    );
    assert!(
        repack_targets("cargo run -- otp repack --targets \"$unset\" --out d\n").is_empty(),
        "a variable with no assignment resolves to nothing: a target this rule cannot see is \
         one it must not accuse"
    );
}

#[test]
fn no_workflow_asks_repack_for_a_target_it_cannot_map() {
    let mut asked = 0usize;
    let mut offenders: Vec<String> = Vec::new();
    for relative in yaml_files_under(".github/workflows") {
        let workflow = yaml(&relative);
        let Some(jobs) = workflow
            .as_mapping_get("jobs")
            .and_then(YamlOwned::as_mapping)
        else {
            continue;
        };
        for (id, job) in jobs {
            let job_id = id.as_str().unwrap_or("<a job id that is not a string>");
            let Some(steps) = job.as_mapping_get("steps").and_then(YamlOwned::as_vec) else {
                continue;
            };
            for step in steps {
                let Some(script) = step.as_mapping_get("run").and_then(YamlOwned::as_str) else {
                    continue;
                };
                for written in repack_targets(script) {
                    for target in expand(job, &written) {
                        asked += 1;
                        if !repackable(&target) {
                            offenders.push(format!(
                                "{relative}: job `{job_id}` runs `otp repack --targets {target}`"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        asked > 0,
        "no workflow runs `ginary otp repack` any more, so this rule has lost its subject"
    );
    assert!(
        offenders.is_empty(),
        "`catalog::upstream_asset` cannot name an upstream asset for these, so the step exits \
         non-zero and takes its job with it — on a workflow that has never run and would first \
         run on a release:\n{}",
        offenders.join("\n")
    );
}
