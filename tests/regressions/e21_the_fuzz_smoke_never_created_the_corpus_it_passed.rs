// SPDX-License-Identifier: MIT OR Apache-2.0
//! The nightly fuzz job passes libFuzzer a corpus directory that git does not
//! track and the job does not create, so every shard exits before it fuzzes
//! anything.
//!
//! **What went wrong.** E19 fixed the sanitizer build, and run
//! [33969332537](https://github.com/P4suta/ginary/actions/runs/33969332537) is
//! the first in which all four targets actually built and launched. All four
//! then exited `1` on libFuzzer's first line:
//!
//! ```text
//! ERROR: The required directory "fuzz/corpus/trailer_parse" does not exist
//! ```
//!
//! `fuzz/corpus/<target>` is where *new* inputs land — the seeds live in
//! `fuzz/seeds/<target>` and are passed second, read-only — so it is empty at
//! checkout, and git tracks no empty directory. `mise.toml`'s `fuzz` task knows
//! that and runs `mkdir -p fuzz/corpus/"$target"` first. The workflow passes
//! the identical two directories and creates neither.
//!
//! **The input.** A clean checkout, which is what every CI run starts from. It
//! cannot be reproduced from a developer's tree, because `mise run fuzz`
//! creates the directory the first time it is run and it is there ever after.
//!
//! **The correct behaviour.** Every caller of the fuzz targets creates the
//! corpus directory it is about to pass, and the two callers are held to one
//! plan — the same targets, the same directories, the same libFuzzer limits —
//! so that a precondition one satisfies and the other does not cannot be
//! written again. Two callers of one command disagreeing about a precondition
//! is the same shape of defect E19 fixed twice.

use crate::common::nightly::{FuzzPlan, TARGET};

/// The directory libFuzzer writes new inputs into, which is the one that has
/// to exist.
const CORPUS: &str = "fuzz/corpus/";

#[test]
fn every_way_of_running_the_fuzzers_creates_the_corpus_it_passes() {
    for plan in [FuzzPlan::from_workflow(), FuzzPlan::from_mise()] {
        let uncreated = plan.uncreated(CORPUS);
        assert_eq!(
            uncreated,
            Vec::<String>::new(),
            "{} passes libFuzzer a corpus directory it never creates. `{CORPUS}{TARGET}` is \
             where new inputs land, so it is empty at checkout and git tracks no empty \
             directory; libFuzzer refuses to start without it, and the job fails before it has \
             fuzzed one byte. It creates {:?} and passes {:?}",
            plan.source,
            plan.creates,
            plan.directories
        );
    }
}

#[test]
fn the_corpus_is_passed_before_the_committed_seeds() {
    // The other half of the precondition, and the reason the corpus directory
    // is the one that has to be created rather than merged into the seeds:
    // libFuzzer writes new inputs into the *first* directory it is given and
    // reads the rest. Passing the committed seeds first would have the fuzzer
    // write into `fuzz/seeds/`, and the corpus this repository reviews would
    // grow by itself.
    for plan in [FuzzPlan::from_workflow(), FuzzPlan::from_mise()] {
        assert_eq!(
            plan.directories,
            vec![
                format!("fuzz/corpus/{TARGET}"),
                format!("fuzz/seeds/{TARGET}")
            ],
            "{} passes its directories in the wrong order or passes others",
            plan.source
        );
    }
}
