# SPDX-License-Identifier: MIT OR Apache-2.0
"""Execute the real assurance scripts with controlled command failures, without Cargo."""
import json
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
GIT_BASH = pathlib.Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
BASH = str(GIT_BASH) if GIT_BASH.is_file() else shutil.which("bash")


@unittest.skipUnless(BASH, "Bash is required to execute the CI assurance scripts")
class AssuranceScriptTests(unittest.TestCase):
    def run_script(self, work, script, **extra):
        file = work / "rehearsal.sh"
        file.write_text('export PATH="/usr/bin:/bin:$PATH"\n' +
                        'if command -v cygpath >/dev/null 2>&1; then GITHUB_WORKSPACE=$(cygpath -u "$GITHUB_WORKSPACE"); fi\n' + script,
                        encoding="utf-8")
        return subprocess.run([BASH, str(file)], cwd=ROOT,
                              env={**os.environ, "GITHUB_WORKSPACE": str(work), **extra},
                              capture_output=True, text=True, timeout=20)

    def test_coverage_reports_after_failures_without_replacing_the_test_verdict(self):
        script = r'''
python3() {
  case " $* " in *' --all-features '*) ;; *) return 76 ;; esac
  printf 'test called\n' >> "$GITHUB_WORKSPACE/calls"
  printf '{"exit_code":%s}\n' "$MOCK_TEST" > "$3/outcomes.json"
  printf 'test log\n' > "$3/tests.log"
  case "${RUSTFLAGS:-}${CARGO_ENCODED_RUSTFLAGS:-}" in *coverage-options=branch*) ;; *) return 77 ;; esac
  case "${RUSTDOCFLAGS:-}${CARGO_ENCODED_RUSTDOCFLAGS:-}" in *coverage-options=branch*) ;; *) return 78 ;; esac
  return "$MOCK_TEST"
}
cargo() {
  if [ "$2" = show-env ]; then
    printf 'export RUSTFLAGS=""\nexport RUSTDOCFLAGS=""\nexport CARGO_LLVM_COV_SHOW_ENV=1\n'
    return 0
  fi
  printf 'report called\n' >> "$GITHUB_WORKSPACE/calls"
  printf 'report diagnostic\n' >&2
  # Match the real 0.8.7/0.9.0 parser, whose help misleadingly lists build
  # selection options that `report` rejects, with or without SHOW_ENV.
  for option in "$@"; do
    case "$option" in
      --all-features|--no-default-features|--features|--branch)
        printf "invalid option '%s' for subcommand 'report'\n" "$option" >&2
        return 1 ;;
    esac
  done
  while [ "$1" != --output-path ]; do shift; done
  printf 'TN:\nSF:src/lib.rs\nLF:10\nLH:%s\nBRF:10\nBRH:%s\nend_of_record\n' "$MOCK_LINE" "$MOCK_BRANCH" > "$2"
  printf 'profile bytes\n' > "$CARGO_LLVM_COV_TARGET_DIR/mock.profraw"
  return "$MOCK_REPORT"
}
source scripts/ci/coverage.sh branches
'''
        for test, report, line, branch, expected in [
            (0, 0, 9, 8, 0), (101, 0, 9, 8, 101), (0, 9, 9, 8, 9),
            (101, 9, 9, 8, 101), (0, 0, 8, 7, 1),
        ]:
            with self.subTest(test=test, report=report, line=line, branch=branch):
                with tempfile.TemporaryDirectory() as temporary:
                    work = pathlib.Path(temporary)
                    result = self.run_script(work, script, MOCK_TEST=str(test), MOCK_REPORT=str(report),
                                             MOCK_LINE=str(line), MOCK_BRANCH=str(branch))
                    self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                    evidence = work / "target/assurance/coverage-branches"
                    record = json.loads((evidence / "run.json").read_text())
                    self.assertEqual(record["test_exit_code"], test)
                    self.assertEqual(record["report_exit_code"], report)
                    self.assertEqual(record["exit_code"], expected)
                    self.assertEqual((work / "calls").read_text().splitlines(), ["test called", "report called"])
                    self.assertIn("report diagnostic", (evidence / "report.log").read_text())
                    self.assertTrue((evidence / "outcomes.json").is_file())
                    self.assertTrue((evidence / "profiles/mock.profraw").is_file())
                    if report == 0:
                        self.assertEqual(record["line_exit_code"], int(line < 9))
                        self.assertEqual(record["branch_exit_code"], int(branch < 8))
                    else:
                        self.assertIsNone(record["line_exit_code"])
                        self.assertIsNone(record["branch_exit_code"])

    def test_fuzz_preserves_output_duration_and_primary_failure(self):
        script = r'''
tee() { command tee "$@"; return "$MOCK_LOG"; }
source scripts/ci/fuzz-evidence.sh
printf 'fuzzer stdout\n'
printf 'fuzzer stderr\n' >&2
exit "$MOCK_FUZZ"
'''
        for fuzz, logger, expected in [(0, 0, 0), (9, 0, 9), (0, 12, 12), (9, 12, 9), (143, 0, 143)]:
            with self.subTest(fuzz=fuzz, logger=logger):
                with tempfile.TemporaryDirectory() as temporary:
                    work = pathlib.Path(temporary)
                    result = self.run_script(work, script, FUZZ_TARGET="trailer_parse",
                                             MOCK_FUZZ=str(fuzz), MOCK_LOG=str(logger))
                    self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                    evidence = work / "target/fuzz-evidence"
                    record = json.loads((evidence / "run.json").read_text())
                    self.assertEqual(record["command_exit_code"], fuzz)
                    self.assertEqual(record["log_exit_code"], logger)
                    self.assertEqual(record["exit_code"], expected)
                    self.assertGreaterEqual(record["elapsed_seconds"], 0)
                    self.assertEqual(record["status"], "interrupted" if fuzz == 143 else "failed" if expected else "successful")
                    log = (evidence / "run.log").read_text()
                    self.assertIn("fuzzer stdout", log)
                    self.assertIn("fuzzer stderr", log)

    def test_smoke_matrix_keeps_failed_builds_and_runtime_evidence(self):
        version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
        script = r'''
docker() {
  case " $* " in
    *' /app 3 a b '*) printf 'args=3 a b\n'; printf 'runtime stderr\n' >&2; return "$MOCK_RUNTIME" ;;
    *) return 0 ;;
  esac
}
gleam() { return 0; }
erl() { return 0; }
export -f docker gleam erl
export GINARY_CATALOG="$GITHUB_WORKSPACE/catalog.json"
export GINARY_STUB_DIR="$GITHUB_WORKSPACE/stubs"
export GINARY_BIN="$GITHUB_WORKSPACE/mock-ginary.sh"
export GINARY_REQUIRE_TOOLCHAIN=1
tee() { command tee "$@"; return "$MOCK_LOG"; }
if [ "$MOCK_WRAPPER" = 1 ]; then
  source scripts/ci/smoke-matrix-evidence.sh
else
  bash scripts/smoke-matrix.sh
fi
'''
        mock = r'''#!/usr/bin/env bash
if [ "$1" = verify ]; then echo 'verified artifact'; exit "$MOCK_VERIFY"; fi
while [ "$1" != --target ]; do shift; done
if command -v cygpath >/dev/null 2>&1; then
  cygpath -m "$PWD" > "$GITHUB_WORKSPACE/work-path"
else
  printf '%s\n' "$PWD" > "$GITHUB_WORKSPACE/work-path"
fi
mkdir -p build/ginary
printf 'artifact bytes\n' > "build/ginary/hello_ffi-$2"
if [ -n "${GINARY_TRACE:-}" ]; then printf 'trace event\n' >> "$GINARY_TRACE"; fi
printf 'build stdout\n'; printf 'build stderr\n' >&2
exit "$MOCK_BUILD"
'''
        for build, verify, runtime, logger, wrapped, expected in [
            (0, 0, 3, 0, 1, 0), (8, 0, 3, 0, 1, 1), (0, 9, 3, 0, 1, 1),
            (0, 0, 0, 0, 1, 1), (0, 0, 3, 12, 1, 12), (8, 0, 3, 12, 1, 1),
            (0, 0, 3, 0, 0, 0),
        ]:
            with self.subTest(build=build, verify=verify, runtime=runtime, logger=logger, wrapped=wrapped):
                with tempfile.TemporaryDirectory() as temporary:
                    work = pathlib.Path(temporary)
                    (work / "catalog.json").write_text('{}')
                    (work / "stubs").mkdir()
                    for target in ("linux-x86_64-musl", "linux-aarch64-musl", "linux-x86_64-gnu"):
                        (work / "stubs" / f"ginary-stub-{version}-{target}").touch()
                    binary = work / "mock-ginary.sh"
                    binary.write_text(mock, encoding="utf-8", newline="\n")
                    binary.chmod(0o755)
                    result = self.run_script(work, script, MOCK_BUILD=str(build), MOCK_VERIFY=str(verify),
                                             MOCK_RUNTIME=str(runtime), MOCK_LOG=str(logger), MOCK_WRAPPER=str(wrapped))
                    self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                    evidence = work / "target/assurance/smoke-matrix"
                    if not wrapped:
                        self.assertFalse(evidence.exists(), "ordinary local runs keep their cleanup behavior")
                        self.assertFalse(pathlib.Path((work / "work-path").read_text().strip()).exists(),
                                         "the default temporary fixture must be removed after completion")
                        continue
                    record = json.loads((evidence / "run.json").read_text())
                    self.assertEqual(record["exit_code"], expected)
                    self.assertEqual(record["log_exit_code"], logger)
                    self.assertGreaterEqual(record["elapsed_seconds"], 0)
                    retained = list(evidence.glob("work.*"))
                    self.assertEqual(len(retained), 1)
                    self.assertIn("build stderr", (retained[0] / "linux-x86_64-musl.build.log").read_text())
                    self.assertTrue((retained[0] / "hello_ffi/build/ginary/hello_ffi-linux-x86_64-musl").is_file())
                    self.assertIn("trace event", (evidence / "trace.ndjson").read_text())
                    if build == 0 and verify == 0:
                        self.assertIn("runtime stderr", (retained[0] / "linux-x86_64-musl.run.log").read_text())
                        runtime_record = json.loads((retained[0] / "linux-x86_64-musl.run.json").read_text())
                        self.assertEqual(runtime_record["exit_code"], runtime)


if __name__ == "__main__":
    unittest.main()
