# SPDX-License-Identifier: MIT OR Apache-2.0
"""Behavioral fixtures for the libtest evidence adapter; no Rust build is needed."""
import importlib.util
import pathlib
import json
import subprocess
import sys
import tempfile
import unittest


class EvidenceTests(unittest.TestCase):
    def adapter(self):
        path = pathlib.Path(__file__).with_name("test-evidence.py")
        self.assertTrue(path.is_file(), "CI must retain structured outcomes, including reported skips")
        spec = importlib.util.spec_from_file_location("test_evidence", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_reports_success_failure_skip_not_run_and_interruption_separately(self):
        adapter = self.adapter()
        listing = """     Running tests/example.rs (target/debug/deps/example)
passed: test
missing_tool: test
failed: test
hanging: test
later: test
"""
        execution = """     Running tests/example.rs (target/debug/deps/example)
test passed ... ok
test missing_tool ... ok
test failed ... FAILED

successes:

---- missing_tool stdout ----
skipping: erl not on PATH

successes:
    missing_tool
    passed

test hanging ... """
        report = adapter.summarize(listing, execution, "interrupted", 130)
        outcomes = {row["name"]: row for row in report["tests"]}
        self.assertEqual({name: row["status"] for name, row in outcomes.items()}, {
            "passed": "successful", "missing_tool": "skipped", "failed": "failed",
            "hanging": "interrupted", "later": "not_run",
        })
        self.assertEqual(outcomes["missing_tool"]["reason"], "erl not on PATH")
        self.assertEqual(report["status"], "interrupted")
        self.assertFalse(report["complete"])

    def test_names_are_scoped_to_harness_and_ignored_is_not_success(self):
        adapter = self.adapter()
        listing = "     Running unittests src/lib.rs (a)\nsame: test\n     Running tests/one.rs (b)\nsame: test\n"
        execution = "     Running unittests src/lib.rs (a)\ntest same ... ok\n     Running tests/one.rs (b)\ntest same ... ignored, external hardware\n"
        report = adapter.summarize(listing, execution, "successful", 0)
        self.assertEqual([row["status"] for row in report["tests"]], ["skipped", "successful"])
        self.assertTrue(report["complete"])

    def test_direct_child_output_does_not_lose_a_test_result(self):
        adapter = self.adapter()
        listing = "     Running tests/child.rs (a)\nchild: test\n"
        execution = "     Running tests/child.rs (a)\ntest child ... direct child stdout\nmore child output\nok\n"
        report = adapter.summarize(listing, execution, "successful", 0)
        self.assertEqual(report["counts"]["successful"], 1)
        self.assertTrue(report["complete"])

    def test_libtest_should_panic_decoration_resolves_to_the_planned_test(self):
        adapter = self.adapter()
        name = "e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence::a_sink_that_answers_interrupted_is_refused_rather_than_hung_on"
        listing = f"     Running tests\\regressions.rs (regressions.exe)\n{name}: test\n"
        execution = f"     Running tests\\regressions.rs (regressions.exe)\ntest {name} - should panic ... ok\n---- {name} stdout ----\nthread '{name}' panicked at tests\\common\\http.rs:846:9\n"
        report = adapter.summarize(listing, execution, "successful", 0)
        self.assertEqual(report["tests"], [{"harness": "tests\\regressions.rs", "name": name, "status": "successful"}])
        self.assertTrue(report["complete"])

    def test_rustdoc_compile_decoration_resolves_to_the_planned_test(self):
        adapter = self.adapter()
        name = "src\\bundle.rs - bundle::build_with_stub_detailed (line 615)"
        listing = f"   Doc-tests ginary\n{name}: test\n"
        execution = f"   Doc-tests ginary\ntest {name} - compile ... ok\n"
        report = adapter.summarize(listing, execution, "successful", 0)
        self.assertEqual(report["tests"], [{"harness": "doc:ginary", "name": name, "status": "successful"}])
        self.assertTrue(report["complete"])

    def test_exact_planned_names_take_priority_over_display_decorations(self):
        adapter = self.adapter()
        listing = "   Doc-tests ginary\nexample: test\nexample - compile: test\n"
        execution = "   Doc-tests ginary\ntest example ... ok\ntest example - compile ... FAILED\n"
        report = adapter.summarize(listing, execution, "failed", 101)
        self.assertEqual([(row["name"], row["status"]) for row in report["tests"]], [
            ("example", "successful"), ("example - compile", "failed"),
        ])
        self.assertTrue(report["complete"])

    def test_decorated_test_interruption_is_assigned_to_the_planned_name(self):
        adapter = self.adapter()
        listing = "     Running tests/panic.rs (a)\nexpected_panic: test\n"
        execution = "     Running tests/panic.rs (a)\ntest expected_panic - should panic ... child stdout\n"
        report = adapter.summarize(listing, execution, "interrupted", 130)
        self.assertEqual(report["tests"], [{"harness": "tests/panic.rs", "name": "expected_panic", "status": "interrupted"}])
        self.assertEqual(report["counts"]["not_run"], 0)
        self.assertFalse(report["complete"])

    def test_real_runner_spools_logs_and_preserves_skips(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            child = root / "child.py"
            child.write_text('''import sys
print("     Running tests/example.rs (fixture)")
if "--list" in sys.argv:
    print("real: test\\nmissing: test")
else:
    print("test real ... ok\\ntest missing ... ok")
    print("successes:\\n---- missing stdout ----\\nskipping: missing optional native runtime")
    print("successes:\\n    real\\n    missing\\ntest result: ok. 2 passed")
''', encoding="utf-8")
            runner = pathlib.Path(__file__).with_name("test-evidence.py")
            result = subprocess.run([sys.executable, str(runner), "--output", str(root / "evidence"),
                                     "--", sys.executable, str(child)], capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
            report = json.loads((root / "evidence/outcomes.json").read_text(encoding="utf-8"))
            self.assertEqual(report["counts"]["successful"], 1)
            self.assertEqual(report["counts"]["skipped"], 1)
            self.assertTrue(report["complete"])
            self.assertIn("missing optional native runtime", (root / "evidence/tests.log").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
