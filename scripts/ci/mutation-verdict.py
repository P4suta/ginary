#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Judge one `cargo mutants --in-diff` run.

A pull request mutates the lines it changed, which is what
`scripts/ci/mutation-diff.sh` asks cargo-mutants for. cargo-mutants then exits
nonzero for a missed mutant *and* for a timeout, and the two are not the same
fact -- nor are the two kinds of timeout:

  * a **test** phase that ran out of time is a mutant the suite did not pass.
    That is the ordinary reading of a hanging mutant and the only reading
    available for one that removes the advance from a loop, where no assertion
    can fire because nothing ever reaches one. It is a kill, counted as `hang`
    rather than folded into `caught` so a reader can see how many there were and
    ask whether the budget is still generous against the baseline;
  * a **build** phase that ran out of time measured nothing at all. That is a
    failure of the run rather than a fact about the tests, and it keeps failing.

A run with no mutants is a pass: a change that touches no mutable line has
nothing for this to say about it, and saying nothing is the honest answer rather
than a skip to explain. A run whose unmutated baseline failed is not a pass at
all, because nothing it reports is about the mutations.
"""

import argparse
import json
import sys
from pathlib import Path

#: The most JSON this will parse. cargo-mutants writes one object per outcome
#: and a diff run has few of them; a file past this is a reason to stop rather
#: than a reason to allocate.
JSON_LIMIT = 128 * 1024 * 1024

#: Every disposition a mutant can end in, in the order a verdict lists them.
#: `hang` and `timeout` are both `Timeout` to cargo-mutants and are two
#: different facts: see `timeout_kind`. `not_run` cannot happen in a diff run
#: and is listed so that a reader comparing two verdicts sees the same shape.
STATUSES = ("caught", "unviable", "missed", "hang", "timeout", "not_run")


def read(path):
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON field: {key}")
            result[key] = value
        return result
    def invalid_constant(value):
        raise ValueError(f"non-finite JSON number: {value}")
    with Path(path).open("rb") as stream:
        encoded = stream.read(JSON_LIMIT + 1)
    if len(encoded) > JSON_LIMIT:
        raise ValueError(f"complete JSON input exceeds {JSON_LIMIT} bytes: {path}; parsing was not attempted")
    return json.loads(encoded.decode("utf-8"),
                      object_pairs_hook=unique_object, parse_constant=invalid_constant)


def valid_phases(result):
    phases = result["phase_results"]
    if not isinstance(phases, list) or not phases:
        return False
    phase_names = [phase["phase"] for phase in phases]
    statuses = [phase["process_status"] for phase in phases]
    def failure(status):
        return (isinstance(status, dict) and set(status) == {"Failure"}
                and type(status["Failure"]) is int and status["Failure"] != 0)
    summary = result["summary"]
    if summary == "Failure" and result["scenario"] == "Baseline":
        def unsuccessful(status):
            return (failure(status) or status == "Other"
                    or (isinstance(status, dict) and set(status) == {"Signalled"}
                        and type(status["Signalled"]) is int and status["Signalled"] > 0))
        return ((phase_names == ["Build"] and unsuccessful(statuses[0]))
                or (phase_names == ["Build", "Test"] and statuses[0] == "Success" and unsuccessful(statuses[1])))
    if summary in ("Success", "MissedMutant"):
        return phase_names == ["Build", "Test"] and statuses == ["Success", "Success"]
    if summary == "CaughtMutant":
        return phase_names == ["Build", "Test"] and statuses[0] == "Success" and failure(statuses[1])
    if summary == "Unviable":
        return phase_names == ["Build"] and failure(statuses[0])
    if summary == "Timeout":
        return ((phase_names == ["Build"] and statuses == ["Timeout"])
                or (phase_names == ["Build", "Test"] and statuses == ["Success", "Timeout"]))
    return False


def timeout_kind(result):
    """Which of the two timeouts this is, by the phase that ran out of time.

    A mutant whose *test* phase exceeds the budget is one the suite did not
    pass: the ordinary reading of a hanging mutant, and the only reading
    available for one that removes the advance from a loop, where no assertion
    can fire because nothing ever reaches one. It is a kill, and is counted as
    `hang` rather than folded into `caught` so that a reader can see how many
    there were and check the budget is still generous against the baseline.

    A mutant whose *build* phase exceeds the budget measured nothing at all.
    That is a failure of the run and not a fact about the tests, so it keeps the
    name `timeout` and keeps failing the gate.
    """
    phases = [phase["phase"] for phase in result["phase_results"]]
    return "hang" if phases == ["Build", "Test"] else "timeout"


def diff_verdict(directory):
    """Read `mutants.out/outcomes.json` and answer with counts and a status."""
    document = read(Path(directory) / "mutants.out/outcomes.json")
    counts = {status: 0 for status in STATUSES}
    baseline = []
    for result in document["outcomes"]:
        if not valid_phases(result):
            raise ValueError(f"summary contradicts actual phases: {result['summary']}")
        if result["scenario"] == "Baseline":
            baseline.append(result["summary"])
            continue
        status = {"CaughtMutant": "caught", "Unviable": "unviable", "MissedMutant": "missed",
                  "Timeout": timeout_kind(result)}.get(result["summary"])
        if status is None:
            raise ValueError(f"unknown mutant summary: {result['summary']}")
        counts[status] += 1
    if baseline != ["Success"]:
        raise ValueError(f"the unmutated baseline did not pass: {baseline}")
    passed = not counts["missed"] and not counts["timeout"]
    return {"schema_version": 1, "mutation_gate_passed": passed, "counts": counts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="the directory `cargo mutants --output` was given")
    options = parser.parse_args()
    try:
        result = diff_verdict(options.output)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"mutation: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0 if result["mutation_gate_passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
