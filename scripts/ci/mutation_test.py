# SPDX-License-Identifier: MIT OR Apache-2.0
"""Adversarial behavior fixtures for native mutation evidence; no Cargo needed."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("mutation_adapter", Path(__file__).with_name("mutation.py"))
ADAPTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ADAPTER)


def candidate(number=1):
    name = f"src/trailer.rs:{number}:5: replace read_{number} -> usize with 0"
    span = {"start": {"line": number, "column": 5}, "end": {"line": number, "column": 6}}
    return {"name": name, "package": "ginary", "file": "src/trailer.rs",
            "function": {"function_name": f"read_{number}", "return_type": "-> usize", "span": span},
            "span": span, "replacement": "0", "genre": "FnValue", "diff": f"--- original\n+++ mutant {number}\n"}


def phases(summary):
    build = {"phase": "Build", "duration": 0.1, "process_status": "Success", "argv": ["cargo", "test", "--no-run"]}
    test = {"phase": "Test", "duration": 0.1, "process_status": "Success", "argv": ["cargo", "test"]}
    if summary == "CaughtMutant":
        test["process_status"] = {"Failure": 101}
    elif summary == "Unviable":
        build["process_status"] = {"Failure": 101}
        return [build]
    elif summary == "Timeout":
        test["process_status"] = "Timeout"
    return [build, test]


class MutationFixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ginary-mutation-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.divisions = {"schema_version": 1, "modules": {"trailer": 1},
                          "max_mutants_per_shard": 13, "build_timeout_seconds": 120, "test_timeout_seconds": 420}
        self.original = [candidate(1), candidate(2)]
        self.canonical = [{"id": "trailer-0", "module": "trailer", "shard": "0/1", "candidates": self.original}]
        self.enriched = [dict(copy.deepcopy(item), applicability={"platforms": ["linux"], "assigned_platform": "linux"})
                         for item in self.original]
        self.bundle = ADAPTER.make_bundle(self.canonical, self.enriched, {"src/trailer.rs": "a" * 64}, self.divisions)
        self.job = self.bundle["jobs"][0]
        self.bundle_path = self.root / "plan.json"
        ADAPTER.write(self.bundle_path, self.bundle)

    def outcome(self, item, summary="CaughtMutant"):
        mutant = {key: copy.deepcopy(value) for key, value in item.items() if key != "diff"}
        number = item["span"]["start"]["line"]
        return {"scenario": {"Mutant": mutant}, "summary": summary, "phase_results": phases(summary),
                "log_path": f"log/mutant-{number}.log", "diff_path": f"diff/mutant-{number}.diff"}

    def records(self):
        return [{"scenario": "Baseline", "summary": "Success", "phase_results": phases("Success"),
                 "log_path": "log/baseline.log", "diff_path": None},
                *(self.outcome(item) for item in self.original)]

    def evidence(self, records=None, directory=None):
        directory = directory or self.root / "job"
        records = self.records() if records is None else records
        ADAPTER.write(directory / "mutants.out/outcomes.json", {"outcomes": records})
        for record in records:
            for key in ("log_path", "diff_path"):
                relative = record.get(key)
                if isinstance(relative, str):
                    path = directory / "mutants.out" / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if key == "log_path":
                        contents = "test detects the changed behavior ... FAILED\n"
                    else:
                        name = record["scenario"]["Mutant"]["name"]
                        contents = next((item["diff"] for item in self.original if item["name"] == name), "--- unknown\n")
                    path.write_text(contents, encoding="utf-8")
        return directory

    def report(self, records=None, status="successful", exit_code=0):
        return ADAPTER.outcome_report(self.bundle, self.job, self.evidence(records), status, exit_code)

    def full_evidence(self):
        evidence_root = self.root / "evidence"
        directory = self.evidence(directory=evidence_root / ("mutants-" + self.job["id"]))
        execution = directory / "execution"
        execution.mkdir()
        for name in ("stdout.log", "stderr.log"):
            (execution / name).write_text("completed mutation execution\n", encoding="utf-8")
        command = ["cargo", "mutants", "--file", "src/trailer.rs", "--re", ADAPTER.exact_regex(self.job["candidates"]),
                   "--timeout", "420", "--build-timeout", "120", "--features", "fault-injection",
                   "--output", str(directory), "--cargo-test-arg=--", "--cargo-test-arg=--show-output"]
        process = {"schema_version": 1, "command": command, "status": "successful", "exit_code": 0,
                   "timeout_seconds": 1740, "elapsed_seconds": 1,
                   "stdout": ADAPTER.tail(execution / "stdout.log"), "stderr": ADAPTER.tail(execution / "stderr.log")}
        ADAPTER.write(execution / "process.json", process)
        ADAPTER.write(directory / "selection.json", {"job": self.job, "plan_sha256": ADAPTER.digest(self.bundle_path),
                                                    "command": command})
        ADAPTER.write(directory / "selection/stdout.log", self.original)
        report = ADAPTER.outcome_report(self.bundle, self.job, directory, "successful", 0)
        report["plan_sha256"] = ADAPTER.digest(self.bundle_path)
        ADAPTER.write(directory / "report.json", report)
        return evidence_root, directory


class PlanTests(MutationFixture):
    def test_complete_json_input_refuses_overflow_duplicate_keys_and_nonfinite_numbers(self):
        path = self.root / "input.json"
        for document in ('{"name":1,"name":2}', '[NaN]', '[Infinity]', '[-Infinity]'):
            path.write_text(document, encoding="utf-8")
            with self.subTest(document=document), self.assertRaises(ValueError):
                ADAPTER.read(path)
        path.write_text('[1,2,3]', encoding="utf-8")
        with mock.patch.object(ADAPTER, "JSON_LIMIT", 6), self.assertRaisesRegex(ValueError, "parsing was not attempted"):
            ADAPTER.read(path)
        with mock.patch.object(ADAPTER, "JSON_LIMIT", 7):
            self.assertEqual(ADAPTER.read(path), [1, 2, 3])

    def test_native_partition_keeps_every_candidate_once_and_original_fields(self):
        self.enriched[1]["applicability"] = {"platforms": ["windows"], "assigned_platform": "windows"}
        bundle = ADAPTER.make_bundle(self.canonical, self.enriched, self.bundle["sources"], self.divisions)
        ADAPTER.validate_bundle(bundle)
        self.assertEqual([(job["platform"], job["candidates"]) for job in bundle["jobs"]],
                         [("linux", [self.original[0]["name"]]), ("windows", [self.original[1]["name"]])])
        self.assertEqual(bundle["canonical"], self.canonical)

    def test_ast_cannot_add_drop_duplicate_or_rewrite_candidates(self):
        variants = [self.enriched[:-1], self.enriched + [self.enriched[0]],
                    self.enriched + [dict(candidate(3), applicability=self.enriched[0]["applicability"])]]
        changed = copy.deepcopy(self.enriched)
        changed[0]["replacement"] = "1"
        variants.append(changed)
        for enriched in variants:
            with self.subTest(enriched=enriched), self.assertRaises((ValueError, KeyError, TypeError)):
                ADAPTER.make_bundle(self.canonical, enriched, self.bundle["sources"], self.divisions)

    def test_assignment_must_name_unique_supported_native_platforms(self):
        for platforms, assigned in [([], "linux"), (["linux", "linux"], "linux"),
                                    (["plan9"], "plan9"), (["windows"], "linux")]:
            with self.subTest(platforms=platforms, assigned=assigned):
                enriched = copy.deepcopy(self.enriched)
                enriched[0]["applicability"] = {"platforms": platforms, "assigned_platform": assigned}
                with self.assertRaises(ValueError):
                    ADAPTER.make_bundle(self.canonical, enriched, self.bundle["sources"], self.divisions)

    def test_empty_plan_is_not_a_successful_zero_job_gate(self):
        divisions = dict(self.divisions, modules={})
        with self.assertRaises(ValueError):
            ADAPTER.validate_bundle(ADAPTER.make_bundle([], [], {}, divisions))

    def test_all_empty_or_over_budget_canonical_plan_is_refused(self):
        for items in ([], [candidate(i) for i in range(1, 15)]):
            with self.subTest(size=len(items)), self.assertRaises(ValueError):
                canonical = [dict(self.canonical[0], candidates=items)]
                enriched = [dict(item, applicability=self.enriched[0]["applicability"]) for item in items]
                ADAPTER.make_bundle(canonical, enriched, self.bundle["sources"], self.divisions)

    def test_invalid_budgets_and_github_matrix_overflow_are_refused(self):
        for field, value in (("max_mutants_per_shard", 14), ("max_mutants_per_shard", True),
                             ("build_timeout_seconds", 0), ("test_timeout_seconds", -1),
                             ("modules", {"trailer": 0})):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                ADAPTER.make_bundle(self.canonical, self.enriched, self.bundle["sources"], dict(self.divisions, **{field: value}))
        canonical = [{"id": f"trailer-{index}", "module": "trailer", "shard": f"{index}/257", "candidates": [candidate(index)]}
                     for index in range(257)]
        enriched = [dict(shard["candidates"][0], applicability=self.enriched[0]["applicability"]) for shard in canonical]
        with self.assertRaisesRegex(ValueError, "256"):
            ADAPTER.make_bundle(canonical, enriched, self.bundle["sources"], dict(self.divisions, modules={"trailer": 257}))

    def test_empty_tail_shard_is_retained_without_a_native_job(self):
        canonical = copy.deepcopy(self.canonical)
        canonical[0]["shard"] = "0/2"
        canonical.append({"id": "trailer-1", "module": "trailer", "shard": "1/2", "candidates": []})
        divisions = dict(self.divisions, modules={"trailer": 2})
        bundle = ADAPTER.make_bundle(canonical, self.enriched, self.bundle["sources"], divisions)
        ADAPTER.validate_bundle(bundle)
        self.assertEqual(len(bundle["canonical"]), 2)
        self.assertEqual(len(bundle["jobs"]), 1)
        self.assertEqual(bundle["canonical"][1]["candidates"], [])
        self.assertEqual(ADAPTER.names(bundle["candidates"]), ADAPTER.names(self.original))

    def test_missing_or_duplicated_canonical_shard_is_refused(self):
        missing = copy.deepcopy(self.bundle)
        missing["divisions"]["modules"]["trailer"] = 2
        with self.assertRaises(ValueError):
            ADAPTER.validate_bundle(missing)
        duplicate = copy.deepcopy(self.bundle)
        duplicate["canonical"].append(copy.deepcopy(duplicate["canonical"][0]))
        with self.assertRaises(ValueError):
            ADAPTER.validate_bundle(duplicate)

    def test_saved_plan_cannot_change_jobs_version_or_cfg_features(self):
        for field, value in (("schema_version", 2), ("cargo_mutants_version", "999.0.0"),
                             ("features", ["fault-injection"]), ("jobs", [])):
            with self.subTest(field=field), self.assertRaises(ValueError):
                changed = copy.deepcopy(self.bundle)
                changed[field] = value
                ADAPTER.validate_bundle(changed)
        changed = copy.deepcopy(self.bundle)
        changed["jobs"][0]["runner"] = "windows-2025"
        with self.assertRaises(ValueError):
            ADAPTER.validate_bundle(changed)

    def test_exact_selection_quotes_regex_syntax_without_matching_neighbors(self):
        ids = ["src/test.rs:7:2: replace f<[u8; 8]> -> Result<T, E> with a+(b)?$",
               r"a.b[c]{2}|\literal^*", "Unicode 日本語 and spaces"]
        expression = ADAPTER.exact_regex(ids)
        for name in ids:
            self.assertIsNotNone(re.fullmatch(expression, name))
            self.assertIsNone(re.fullmatch(expression, name + " suffix"))
            self.assertIsNone(re.fullmatch(expression, "prefix " + name))
        self.assertIsNone(re.fullmatch(expression, "aXbcccc"))
        for invalid in ([], ["same", "same"], [""]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                ADAPTER.exact_regex(invalid)

    def test_source_addition_deletion_and_byte_changes_are_detected(self):
        source = self.root / "source"
        (source / "src").mkdir(parents=True)
        file = source / "src/lib.rs"
        file.write_bytes(b"original")
        snapshot = ADAPTER.source_manifest(source)
        ADAPTER.verify_sources(source, snapshot)
        for action in ("change", "delete", "add"):
            file.write_bytes(b"original")
            if action == "change":
                file.write_bytes(b"changed")
            elif action == "delete":
                file.unlink()
            else:
                (source / "src/new.rs").write_bytes(b"added")
            with self.subTest(action=action), self.assertRaises(ValueError):
                ADAPTER.verify_sources(source, snapshot)


class OutcomeTests(MutationFixture):
    def test_failed_baseline_keeps_its_status_and_log_with_all_mutations_unrun(self):
        baseline = self.records()[0]
        baseline["summary"] = "Failure"
        baseline["phase_results"][1]["process_status"] = {"Failure": 101}
        report = self.report([baseline], "failed", 4)
        self.assertEqual(report["baseline"], ["Failure"])
        self.assertEqual(report["errors"], [])
        self.assertIn("mutants.out/log/baseline.log", report["evidence"])
        self.assertEqual(report["counts"]["not_run"], 2)
        self.assertFalse(report["complete"])
        self.assertFalse(report["mutation_gate_passed"])

    def test_valid_caught_and_compile_rejected_results_remain_distinct(self):
        records = self.records()
        records[2] = self.outcome(self.original[1], "Unviable")
        report = self.report(records)
        self.assertTrue(report["mutation_gate_passed"], report)
        self.assertEqual(report["counts"], {"caught": 1, "unviable": 1, "missed": 0, "timeout": 0, "not_run": 0})

    def test_missing_or_partial_outcomes_remain_incomplete_with_unrun_candidates(self):
        missing = ADAPTER.outcome_report(self.bundle, self.job, self.root / "missing", "failed", 1)
        self.assertEqual(missing["counts"]["not_run"], 2)
        self.assertFalse(missing["mutation_gate_passed"])
        report = self.report(self.records()[:-1])
        self.assertEqual(report["counts"]["not_run"], 1)
        self.assertFalse(report["complete"])

    def test_unknown_duplicate_or_changed_outcomes_are_never_success(self):
        variants = []
        duplicate = self.records()
        duplicate.append(copy.deepcopy(duplicate[-1]))
        variants.append(duplicate)
        unknown = self.records()
        unknown[1] = self.outcome(candidate(99))
        variants.append(unknown)
        for key, value in (("name", "unknown"), ("file", "src/other.rs"), ("replacement", "9"),
                           ("genre", "Other"), ("function", None), ("span", None), ("package", "another-crate")):
            changed = self.records()
            changed[1]["scenario"]["Mutant"][key] = value
            variants.append(changed)
        for records in variants:
            with self.subTest(records=records):
                report = self.report(records)
                self.assertFalse(report["mutation_gate_passed"], report)
                self.assertTrue(report["errors"], report)

    def test_unknown_summary_or_malformed_documents_are_refused(self):
        records = self.records()
        records[1]["summary"] = "NewUnrecognizedSuccess"
        self.assertFalse(self.report(records)["mutation_gate_passed"])
        directory = self.evidence()
        for contents in ('{"outcomes":', '{"outcomes":{}}', '{"outcomes":[null]}'):
            with self.subTest(contents=contents):
                (directory / "mutants.out/outcomes.json").write_text(contents, encoding="utf-8")
                report = ADAPTER.outcome_report(self.bundle, self.job, directory, "successful", 0)
                self.assertFalse(report["mutation_gate_passed"])
                self.assertTrue(report["errors"])

    def test_missing_duplicate_or_failed_baseline_is_not_complete(self):
        for modification in ("missing", "duplicate", "failed"):
            records = self.records()
            if modification == "missing":
                records.pop(0)
            elif modification == "duplicate":
                records.insert(0, copy.deepcopy(records[0]))
            else:
                records[0]["summary"] = "Failure"
            with self.subTest(modification=modification):
                report = self.report(records)
                self.assertFalse(report["complete"])
                self.assertFalse(report["mutation_gate_passed"])

    def test_summary_must_agree_with_actual_build_and_test_phase_verdicts(self):
        for position, invalid in ((0, []), (0, phases("CaughtMutant")),
                                  (1, []), (1, phases("Success")), (1, phases("Unviable"))):
            records = self.records()
            records[position]["phase_results"] = invalid
            with self.subTest(position=position, phases=invalid):
                self.assertFalse(self.report(records)["mutation_gate_passed"])

    def test_unknown_zero_and_boolean_process_failures_cannot_count_as_caught(self):
        for status in ("NewStatus", "Other", {"Failure": 0}, {"Failure": False}, {"Signalled": 9}):
            records = self.records()
            records[1]["phase_results"][1]["process_status"] = status
            with self.subTest(status=status):
                self.assertFalse(self.report(records)["mutation_gate_passed"])

    def test_missed_timeout_and_process_failures_never_pass(self):
        for summary in ("MissedMutant", "Timeout"):
            records = self.records()
            records[1] = self.outcome(self.original[0], summary)
            with self.subTest(summary=summary):
                report = self.report(records)
                self.assertFalse(report["mutation_gate_passed"])
                self.assertEqual(report["counts"]["missed" if summary == "MissedMutant" else "timeout"], 1)
        for status, code in (("failed", 1), ("timeout", -9), ("interrupted", None),
                             ("not_run", None), ("successful", 7), ("spawn_failed", None)):
            with self.subTest(status=status, code=code):
                self.assertFalse(self.report(status=status, exit_code=code)["mutation_gate_passed"])

    def test_missing_or_empty_log_and_diff_is_evidence_failure(self):
        for relative in ("log/baseline.log", "log/mutant-1.log", "diff/mutant-1.diff"):
            for action in ("delete", "empty"):
                directory = self.evidence()
                path = directory / "mutants.out" / relative
                path.unlink() if action == "delete" else path.write_bytes(b"")
                with self.subTest(relative=relative, action=action):
                    report = ADAPTER.outcome_report(self.bundle, self.job, directory, "successful", 0)
                    self.assertFalse(report["mutation_gate_passed"])
                    self.assertTrue(report["errors"])

    def test_evidence_paths_cannot_escape_the_recorded_job(self):
        directory = self.evidence()
        for relative in ("../../outside.log", "C:/outside.log", r"log\windows.log"):
            records = self.records()
            records[1]["log_path"] = relative
            ADAPTER.write(directory / "mutants.out/outcomes.json", {"outcomes": records})
            with self.subTest(relative=relative):
                self.assertFalse(ADAPTER.outcome_report(self.bundle, self.job, directory, "successful", 0)["mutation_gate_passed"])

    def test_recorded_diff_must_describe_the_planned_candidate(self):
        directory = self.evidence()
        (directory / "mutants.out/diff/mutant-1.diff").write_text(self.original[1]["diff"], encoding="utf-8")
        report = ADAPTER.outcome_report(self.bundle, self.job, directory, "successful", 0)
        self.assertFalse(report["mutation_gate_passed"], "a nonempty diff for another candidate is not the planned mutation")
        self.assertTrue(report["errors"])


class ReconciliationTests(MutationFixture):
    def test_malformed_process_record_still_produces_accounted_failure(self):
        root, directory = self.full_evidence()
        for malformed in ([], None, "successful"):
            ADAPTER.write(directory / "execution/process.json", malformed)
            with self.subTest(malformed=malformed):
                result = ADAPTER.reconcile(self.bundle_path, root)
                self.assertFalse(result["mutation_gate_passed"])
                self.assertEqual(sum(result["counts"].values()), len(self.original))

    def test_valid_artifacts_reconcile_to_every_planned_candidate(self):
        root, _ = self.full_evidence()
        report = ADAPTER.reconcile(self.bundle_path, root)
        self.assertTrue(report["mutation_gate_passed"], report)
        self.assertEqual(report["counts"]["caught"], 2)
        self.assertEqual(report["planned_candidates"], 2)

    def test_missing_extra_or_wrongly_named_job_artifacts_fail(self):
        root, directory = self.full_evidence()
        for action in ("missing", "extra", "rename"):
            with self.subTest(action=action):
                if action == "missing":
                    renamed = directory.with_name("hidden")
                    directory.rename(renamed)
                    self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])
                    renamed.rename(directory)
                else:
                    extra = root / ("unexpected" if action == "extra" else "mutants-wrong-job")
                    extra.mkdir()
                    self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])
                    extra.rmdir()

    def test_absent_job_retains_all_of_its_candidates_as_not_run(self):
        report = ADAPTER.reconcile(self.bundle_path, self.root / "no-evidence")
        self.assertFalse(report["mutation_gate_passed"])
        self.assertEqual(report["counts"]["not_run"], 2)
        self.assertEqual(sum(report["counts"].values()), report["planned_candidates"])

    def test_failed_job_keeps_its_actual_missed_and_caught_counts(self):
        root, directory = self.full_evidence()
        records = self.records()
        records[1] = self.outcome(self.original[0], "MissedMutant")
        self.evidence(records, directory)
        process = ADAPTER.read(directory / "execution/process.json")
        process.update(status="failed", exit_code=2)
        ADAPTER.write(directory / "execution/process.json", process)
        report = ADAPTER.outcome_report(self.bundle, self.job, directory, "failed", 2)
        report["plan_sha256"] = ADAPTER.digest(self.bundle_path)
        ADAPTER.write(directory / "report.json", report)
        reconciled = ADAPTER.reconcile(self.bundle_path, root)
        self.assertFalse(reconciled["mutation_gate_passed"])
        self.assertEqual(reconciled["counts"]["missed"], 1)
        self.assertEqual(reconciled["counts"]["caught"], 1)
        self.assertEqual(sum(reconciled["counts"].values()), reconciled["planned_candidates"])

    def test_modified_plan_or_saved_candidate_counts_and_hashes_fail(self):
        root, directory = self.full_evidence()
        original = ADAPTER.read(directory / "report.json")
        for field, value in (("plan_sha256", "0" * 64), ("job", "trailer-99-linux"),
                             ("candidates", []), ("counts", {}), ("evidence", {}), ("platform", "windows")):
            changed = dict(original, **{field: value})
            ADAPTER.write(directory / "report.json", changed)
            with self.subTest(field=field):
                self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])
        ADAPTER.write(directory / "report.json", original)
        changed_plan = copy.deepcopy(self.bundle)
        changed_plan["sources"]["src/trailer.rs"] = "b" * 64
        ADAPTER.write(self.bundle_path, changed_plan)
        self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])

    def test_changed_raw_log_after_report_is_not_silently_rehashed_success(self):
        root, directory = self.full_evidence()
        (directory / "mutants.out/log/mutant-1.log").write_text("different log\n", encoding="utf-8")
        self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])

    def test_success_report_cannot_override_failed_or_missing_process_evidence(self):
        root, directory = self.full_evidence()
        process_path = directory / "execution/process.json"
        original = ADAPTER.read(process_path)
        for action in ("failure", "nonzero", "missing", "missing_stdout"):
            ADAPTER.write(process_path, original)
            with self.subTest(action=action):
                if action == "missing":
                    process_path.unlink()
                elif action == "missing_stdout":
                    (directory / "execution/stdout.log").unlink()
                else:
                    process = dict(original, status="failed" if action == "failure" else "successful", exit_code=17)
                    ADAPTER.write(process_path, process)
                self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])

    def test_matching_forged_command_records_cannot_replace_the_planned_execution(self):
        root, directory = self.full_evidence()
        process = ADAPTER.read(directory / "execution/process.json")
        selection = ADAPTER.read(directory / "selection.json")
        for command in (["true"], ["cargo", "mutants", "--baseline", "skip"],
                        ["cargo", "mutants", "--re", "only-one-candidate"]):
            process["command"] = selection["command"] = command
            ADAPTER.write(directory / "execution/process.json", process)
            ADAPTER.write(directory / "selection.json", selection)
            with self.subTest(command=command):
                self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])

    def test_selection_discovery_cannot_drop_duplicate_or_rewrite_an_assigned_candidate(self):
        root, directory = self.full_evidence()
        changed = copy.deepcopy(self.original)
        changed[0]["replacement"] = "999"
        for selected in (self.original[:-1], self.original + [self.original[0]], changed):
            ADAPTER.write(directory / "selection/stdout.log", selected)
            with self.subTest(selected=selected):
                self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])

    def test_execution_capture_tail_and_cleanup_cannot_disagree_with_success(self):
        root, directory = self.full_evidence()
        original = ADAPTER.read(directory / "execution/process.json")
        for alteration in ("tail", "byte_count", "cleanup"):
            process = copy.deepcopy(original)
            if alteration == "tail":
                process["stdout"]["tail"] = "unrecorded replacement"
            elif alteration == "byte_count":
                process["stderr"]["bytes"] += 1
            else:
                process["cleanup_error"] = "child could not be reaped"
            ADAPTER.write(directory / "execution/process.json", process)
            with self.subTest(alteration=alteration):
                self.assertFalse(ADAPTER.reconcile(self.bundle_path, root)["mutation_gate_passed"])


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ginary-mutation-process-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_real_failure_retains_both_streams_and_exit_code(self):
        record = ADAPTER.execute([sys.executable, "-c", "import sys; print('stdout evidence'); print('stderr evidence',file=sys.stderr); sys.exit(7)"], self.root, 10)
        self.assertEqual((record["status"], record["exit_code"]), ("failed", 7))
        self.assertIn("stdout evidence", record["stdout"]["tail"])
        self.assertIn("stderr evidence", record["stderr"]["tail"])
        self.assertEqual(ADAPTER.read(self.root / "process.json"), record)

    def test_child_evidence_environment_is_scoped_without_changing_the_parent(self):
        original = os.environ.get("GINARY_TEST_EVIDENCE_DIR")
        expected = str(self.root / "test-failures")
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "stale-target", "CARGO_BUILD_TARGET_DIR": "also-stale"}):
            environment = ADAPTER.mutation_environment(self.root)
            record = ADAPTER.execute([sys.executable, "-c", "import os; assert 'CARGO_TARGET_DIR' not in os.environ; assert 'CARGO_BUILD_TARGET_DIR' not in os.environ; print(os.environ['GINARY_TEST_EVIDENCE_DIR'])"],
                                     self.root, 10, env=environment)
            self.assertEqual(os.environ["CARGO_TARGET_DIR"], "stale-target")
            self.assertEqual(os.environ["CARGO_BUILD_TARGET_DIR"], "also-stale")
        self.assertEqual(record["status"], "successful")
        self.assertEqual(record["stdout"]["tail"].strip(), expected)
        self.assertEqual(os.environ.get("GINARY_TEST_EVIDENCE_DIR"), original)

    def test_real_timeout_preserves_partial_output_and_reaps_the_child(self):
        marker = self.root / "survived.txt"
        script = "import pathlib,time; print('before timeout',flush=True); time.sleep(1.5); pathlib.Path(" + repr(str(marker)) + ").write_text('alive')"
        started = time.monotonic()
        record = ADAPTER.execute([sys.executable, "-u", "-c", script], self.root, 0.4)
        self.assertEqual(record["status"], "timeout")
        self.assertIsNotNone(record["exit_code"])
        self.assertNotIn("cleanup_error", record)
        self.assertIn("before timeout", record["stdout"]["tail"])
        self.assertLess(time.monotonic() - started, 8)
        time.sleep(1.3)
        self.assertFalse(marker.exists(), "the timed-out child must not continue writing after execute returns")

    def test_timeout_also_stops_a_descendant_that_inherits_output_handles(self):
        ready = self.root / "grandchild-started.txt"
        survived = self.root / "grandchild-survived.txt"
        grandchild = ("import pathlib,time,sys; pathlib.Path(" + repr(str(ready)) + ").write_text('started'); "
                      "print('grandchild stderr before timeout',file=sys.stderr,flush=True); time.sleep(2); pathlib.Path("
                      + repr(str(survived)) + ").write_text('alive')")
        parent = ("import subprocess,sys,time; subprocess.Popen([sys.executable,'-u','-c'," + repr(grandchild)
                  + "]); print('parent stdout before timeout',flush=True); time.sleep(30)")
        started = time.monotonic()
        record = ADAPTER.execute([sys.executable, "-u", "-c", parent], self.root, 1)
        self.assertEqual(record["status"], "timeout")
        self.assertNotIn("cleanup_error", record)
        self.assertTrue(ready.is_file(), "the descendant must actually start for its cleanup to be tested")
        self.assertIn("parent stdout before timeout", record["stdout"]["tail"])
        self.assertIn("grandchild stderr before timeout", record["stderr"]["tail"])
        self.assertLess(time.monotonic() - started, 8)
        time.sleep(1.3)
        self.assertFalse(survived.exists(), "the process-tree cleanup must also reap inherited-output descendants")

    def test_spawn_failure_is_explicit_and_has_a_durable_record(self):
        record = ADAPTER.execute([str(self.root / "missing-executable")], self.root, 1)
        self.assertEqual(record["status"], "spawn_failed")
        self.assertIsNone(record["exit_code"])
        self.assertTrue(record["reason"])
        self.assertEqual(ADAPTER.read(self.root / "process.json"), record)

    def test_refused_tree_termination_is_reported_after_best_effort_direct_child_cleanup(self):
        class OwnedChild:
            pid = 12345
            killed = False
            waited = False

            def poll(self):
                return -9 if self.killed else None

            def kill(self):
                self.killed = True

            def wait(self, timeout):
                self.waited = True
                return -9

        child = OwnedChild()
        failure = mock.Mock(returncode=1, stdout="", stderr="ERROR: Access denied")
        with mock.patch.object(ADAPTER.os, "name", "nt"), mock.patch.object(ADAPTER.subprocess, "run", return_value=failure):
            with self.assertRaises(OSError):
                ADAPTER.kill_tree(child)
        self.assertTrue(child.killed, "failure to kill the tree still requires best-effort direct-child cleanup")
        self.assertTrue(child.waited, "the directly owned child must be reaped before the error is reported")

    def test_large_output_is_spooled_completely_but_diagnostic_tail_is_bounded(self):
        size = ADAPTER.TAIL_LIMIT + 8192
        record = ADAPTER.execute([sys.executable, "-c", f"import sys; sys.stdout.write('x'*{size}); sys.stdout.flush()"], self.root, 10)
        self.assertEqual(record["status"], "successful")
        self.assertEqual((self.root / "stdout.log").stat().st_size, size)
        self.assertEqual(record["stdout"]["bytes"], size)
        self.assertEqual(record["stdout"]["omitted_bytes"], 8192)
        self.assertEqual(len(record["stdout"]["tail"]), ADAPTER.TAIL_LIMIT)


if __name__ == "__main__":
    unittest.main()
