// SPDX-License-Identifier: MIT OR Apache-2.0
//! Six of the seven nightly mutation shards are asked for hours of work
//! against a ninety-minute cap, so they are killed part way through and the
//! pass has never once produced an answer.
//!
//! **What went wrong.** E19 fixed the baseline — run
//! [33969332537](https://github.com/P4suta/ginary/actions/runs/33969332537) is
//! the first in which every shard printed `ok Unmutated baseline` — and that
//! is what made the second problem visible. The shards are one per module and
//! each takes the whole module:
//!
//! ```text
//! matrix: module: [trailer, payload, cache, closure, appfile, launch, verify]
//! run: cargo mutants --file "src/${{ matrix.module }}.rs" --features fault-injection
//! timeout-minutes: 90
//! ```
//!
//! The one shard that finished, `trailer`, reported `28 mutants tested in 50m`
//! — 107 seconds a mutant, that shard's share of the baseline included. The
//! others announced 91 to 207 mutants, which is between three and seven hours.
//! `appfile` died on `The runner has received a shutdown signal`; `cache`,
//! `closure`, `launch`, `payload` and `verify` were `cancelled` at the cap,
//! each having reported some survivors and never a total. A `cancelled` job is
//! not a result: it neither passes nor fails, so the pass has never gated
//! anything, and a survivor it happened to print was noticed only because
//! somebody read the log.
//!
//! Nothing caps one mutant either. `cargo mutants` runs the suite per mutant,
//! and a mutant that removes a loop's exit condition hangs until something
//! kills it — which, with no `--timeout`, is the job's own budget. One such
//! mutant eats a whole shard.
//!
//! **The input.** Every scheduled run since the job was written.
//!
//! **The correct behaviour.** A shard is asked for work it can finish inside
//! the budget it is given, and what one mutant may cost is capped so that one
//! hang cannot spend the rest. What the pass covers and what it does not is
//! then a statement somebody can check, rather than whatever fitted before the
//! runner was killed. The measured side of the argument is
//! `tests/fixtures/nightly/mutants-measured.json`; a gate that cannot finish
//! is not a gate.

use std::collections::BTreeMap;

use crate::common::nightly::{MEASURED_MUTANTS, measured_mutants, mutants_plan};

#[test]
fn every_mutation_shard_fits_inside_the_budget_it_is_given() {
    let plan = mutants_plan();
    let measured = measured_mutants();

    let mut overruns: Vec<String> = Vec::new();
    for shard in &plan.shards {
        let Some(total) = measured.modules.get(&shard.module) else {
            continue;
        };
        let mutants = total.div_ceil(shard.shards.max(1));
        let minutes = measured.minutes_for(mutants);
        if minutes > plan.timeout_minutes {
            overruns.push(format!(
                "{} takes {mutants} of `src/{}.rs`'s {total} mutants, which is about {minutes} \
                 minutes against a budget of {}",
                shard.row, shard.module, plan.timeout_minutes
            ));
        }
    }

    assert_eq!(
        overruns,
        Vec::<String>::new(),
        "a shard that cannot finish is cancelled at the cap, and a cancelled job neither passes \
         nor fails — so the pass gates nothing at all. The costs are measured, from the run named \
         in {MEASURED_MUTANTS}: {} seconds a mutant and {} minutes to the baseline",
        measured.seconds_per_mutant,
        measured.baseline_minutes
    );
}

#[test]
fn every_mutation_shard_caps_what_one_mutant_may_cost() {
    let uncapped: Vec<String> = mutants_plan()
        .shards
        .iter()
        .filter(|shard| shard.timeout.is_none())
        .map(|shard| shard.row.clone())
        .collect();

    assert_eq!(
        uncapped,
        Vec::<String>::new(),
        "`cargo mutants` runs the suite once per mutant, and a mutant that removes a loop's exit \
         condition does not end. With no `--timeout` the only thing that stops it is the job's \
         own budget, so one hang costs the whole shard and reports nothing about the other \
         mutants it never reached"
    );
}

#[test]
fn the_shards_of_a_module_cover_all_of_it() {
    // The pass may be divided; it may not quietly shrink. A `--shard 0/4` with
    // no 1, 2 and 3 beside it is three quarters of a module that nothing
    // mutates, and it would make the budget assertion above pass by testing
    // less rather than by dividing the work.
    //
    // `cargo mutants` numbers its shards from zero — `--shard 1/1` is refused
    // with `shard k must be less than n` — so the division `--shard k/n` runs
    // is `0 .. n-1` and that is what a complete list of shards is. See
    // `docs/dev/log/E21.md`, "GREEN / test corrections".
    let mut seen: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut divisions: BTreeMap<String, u64> = BTreeMap::new();
    for shard in mutants_plan().shards {
        seen.entry(shard.module.clone())
            .or_default()
            .push(shard.index);
        divisions.insert(shard.module, shard.shards);
    }

    for (module, indices) in &mut seen {
        indices.sort_unstable();
        let divided = divisions.get(module).copied().unwrap_or(1);
        assert_eq!(
            *indices,
            (0..divided).collect::<Vec<_>>(),
            "`src/{module}.rs` is divided into {divided} shards and the matrix runs {indices:?} \
             of them. `cargo mutants` numbers a division from zero, so the complete list is \
             `0/{divided}` to `{}/{divided}`. Every part of a module a shard list divides has to \
             be run by one of the shards, or the module is only partly mutated and the pass says \
             so nowhere",
            divided.saturating_sub(1)
        );
    }

    let mutated: Vec<String> = seen.keys().cloned().collect();
    let mut measured: Vec<String> = measured_mutants().modules.keys().cloned().collect();
    measured.sort();
    assert_eq!(
        mutated, measured,
        "and the modules the matrix mutates are the modules {MEASURED_MUTANTS} was measured \
         over. A module dropped from the matrix is a module nothing mutates; one added without \
         being measured has no budget anybody has checked"
    );
}
