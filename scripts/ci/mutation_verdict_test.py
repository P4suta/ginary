#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""What `mutation-verdict.py` calls a pass, and what it refuses."""

import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "mutation_verdict", ROOT / "scripts/ci/mutation-verdict.py")
VERDICT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERDICT)


def phases(summary):
    """The phase list cargo-mutants writes for each outcome.

    `Timeout` has two shapes and they mean different things: a test phase that
    ran out of time is a mutant the suite did not pass, and a build phase that
    ran out of time measured nothing. `BuildTimeout` is this fixture's name for
    the second; cargo-mutants calls both of them `Timeout` and tells them apart
    by exactly this list.
    """
    build = {"phase": "Build", "duration": 0.1, "process_status": "Success",
             "argv": ["cargo", "test", "--no-run"]}
    test = {"phase": "Test", "duration": 0.1, "process_status": "Success",
            "argv": ["cargo", "test"]}
    if summary == "CaughtMutant":
        test["process_status"] = {"Failure": 101}
    elif summary == "Unviable":
        build["process_status"] = {"Failure": 101}
        return [build]
    elif summary == "BuildTimeout":
        build["process_status"] = "Timeout"
        return [build]
    elif summary == "Timeout":
        test["process_status"] = "Timeout"
    return [build, test]


def mutant(summary, line=1):
    return {"scenario": {"Mutant": {"name": f"src/trailer.rs:{line}:1: replace x with y"}},
            "summary": "Timeout" if summary == "BuildTimeout" else summary,
            "phase_results": phases(summary),
            "log_path": f"log/mutant-{line}.log", "diff_path": f"diff/mutant-{line}.diff"}


def baseline(summary="Success"):
    record = {"scenario": "Baseline", "summary": summary,
              "phase_results": phases("Success"),
              "log_path": "log/baseline.log", "diff_path": None}
    if summary == "Failure":
        record["phase_results"][1]["process_status"] = {"Failure": 101}
    return record


class VerdictTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ginary-verdict-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)

    def verdict(self, *records):
        out = self.root / "mutants.out"
        out.mkdir(parents=True, exist_ok=True)
        (out / "outcomes.json").write_text(json.dumps({"outcomes": list(records)}),
                                           encoding="utf-8")
        return VERDICT.diff_verdict(self.root)

    def test_each_outcome_is_counted_under_its_own_name(self):
        """And the two timeouts are not one name.

        A mutant that stops a loop advancing cannot be caught by an assertion,
        because nothing reaches one; the suite failing to terminate *is* the
        detection, and mutation testing has always counted that as a kill. A
        build that ran out of time measured nothing, which is a different fact
        and stays a failure.
        """
        for summary, status, passes in (("CaughtMutant", "caught", True),
                                        ("Unviable", "unviable", True),
                                        ("Timeout", "hang", True),
                                        ("MissedMutant", "missed", False),
                                        ("BuildTimeout", "timeout", False)):
            with self.subTest(summary=summary):
                result = self.verdict(baseline(), mutant(summary))
                self.assertEqual(result["mutation_gate_passed"], passes, result)
                self.assertEqual(result["counts"][status], 1, result)
                self.assertEqual(sum(result["counts"].values()), 1, result)

    def test_a_diff_that_touches_no_mutable_line_is_a_pass(self):
        result = self.verdict(baseline())
        self.assertTrue(result["mutation_gate_passed"])
        self.assertEqual(sum(result["counts"].values()), 0)
        self.assertEqual(sorted(result["counts"]), sorted(VERDICT.STATUSES))

    def test_a_run_whose_baseline_failed_says_nothing_about_the_mutants(self):
        with self.assertRaisesRegex(ValueError, "baseline"):
            self.verdict(baseline("Failure"), mutant("CaughtMutant"))

    def test_a_summary_its_phases_contradict_is_refused(self):
        """cargo-mutants is the source of both, so disagreement is corruption."""
        broken = mutant("CaughtMutant")
        broken["phase_results"][1]["process_status"] = "Success"
        with self.assertRaisesRegex(ValueError, "contradicts"):
            self.verdict(baseline(), broken)

    def test_an_unknown_summary_is_refused_rather_than_ignored(self):
        unknown = mutant("CaughtMutant")
        unknown["summary"] = "Marvellous"
        with self.assertRaises(ValueError):
            self.verdict(baseline(), unknown)

    def test_a_missing_outcomes_file_is_an_error_and_not_an_empty_pass(self):
        with self.assertRaises(OSError):
            VERDICT.diff_verdict(self.root / "absent")


if __name__ == "__main__":
    unittest.main()
