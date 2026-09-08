#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Retain test outcomes and report data even when instrumentation exposes a failure.
set -euo pipefail
mode=${1:?usage: coverage.sh lines|branches}
case "$mode" in lines|branches) ;; *) echo "unknown coverage mode: $mode" >&2; exit 2 ;; esac
root=${GITHUB_WORKSPACE:-$PWD}
evidence="$root/target/assurance/coverage-$mode"
mkdir -p "$evidence"
printf '{"status":"running","complete":false}\n' > "$evidence/run.json"
trap 'status=$?; printf "{\"schema_version\":1,\"status\":\"failed\",\"phase\":\"setup\",\"complete\":false,\"exit_code\":%s}\n" "$status" > "$evidence/run.json"' EXIT
export CARGO_TARGET_DIR
CARGO_TARGET_DIR=$(mktemp -d "$root/target/coverage-$mode.XXXXXX")
export CARGO_LLVM_COV_TARGET_DIR="$CARGO_TARGET_DIR"
export GINARY_TEST_EVIDENCE_DIR="$evidence/evidence"
doc_flags=()
if [ "$mode" = branches ]; then doc_flags=(--doctests); fi
coverage_env=$(cargo llvm-cov show-env --sh "${doc_flags[@]}" 2> "$evidence/setup.log")
eval "$coverage_env"
if [ "$mode" = branches ]; then
  # show-env has no --branch option, including in the local 0.8.7 installation.
  if [ "${CARGO_ENCODED_RUSTFLAGS+x}" ]; then
    export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS}"$'\x1f''-Zcoverage-options=branch'
  else
    export RUSTFLAGS="${RUSTFLAGS:-} -Zcoverage-options=branch"
  fi
  if [ "${CARGO_ENCODED_RUSTDOCFLAGS+x}" ]; then
    export CARGO_ENCODED_RUSTDOCFLAGS="${CARGO_ENCODED_RUSTDOCFLAGS}"$'\x1f''-Zcoverage-options=branch'
  else
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zcoverage-options=branch"
  fi
fi
trap - EXIT
set +e
python3 scripts/ci/test-evidence.py --output "$evidence" -- \
  cargo test --all-features --workspace --locked --no-fail-fast
test_status=$?
lcov="$evidence/coverage.lcov"
# Features select the tests above. The report parser rejects --all-features
# despite listing it in help; report reads the already-instrumented objects.
cargo llvm-cov report "${doc_flags[@]}" --locked --lcov --output-path "$lcov" \
  2>&1 | tee "$evidence/report.log"
report_codes=("${PIPESTATUS[@]}")
report_status=${report_codes[0]}
if [ "$report_status" -eq 0 ]; then report_status=${report_codes[1]}; fi
line_status=null
branch_status=null
if [ "$report_status" -eq 0 ]; then
  bash scripts/ci/coverage-gate.sh "$lcov" 90 2>&1 | tee "$evidence/lines.log"
  gate_codes=("${PIPESTATUS[@]}")
  line_status=${gate_codes[0]}
  if [ "$line_status" -eq 0 ]; then line_status=${gate_codes[1]}; fi
  if [ "$mode" = branches ]; then
    bash scripts/ci/coverage-gate.sh "$lcov" 80 branches 2>&1 | tee "$evidence/branches.log"
    gate_codes=("${PIPESTATUS[@]}")
    branch_status=${gate_codes[0]}
    if [ "$branch_status" -eq 0 ]; then branch_status=${gate_codes[1]}; fi
  fi
fi
# Profiles remain useful when report generation fails; the build directory is
# named for rerendering on the same runner, without uploading all dependencies.
mkdir -p "$evidence/profiles"
find "$CARGO_LLVM_COV_TARGET_DIR" -maxdepth 1 -type f \
  \( -name '*.profraw' -o -name '*.profdata' \) -exec cp {} "$evidence/profiles/" \;
profile_status=$?
status=$test_status
for result in "$report_status" "$line_status" "$branch_status" "$profile_status"; do
  if [ "$status" -eq 0 ] && [ "$result" != null ] && [ "$result" -ne 0 ]; then status=$result; fi
done
verdict=failed
if [ "$status" -eq 0 ]; then verdict=successful; fi
if ! printf '%s\n' "$CARGO_LLVM_COV_TARGET_DIR" > "$evidence/build-directory.txt"; then
  if [ "$status" -eq 0 ]; then status=1; verdict=failed; fi
fi
if ! printf '{"schema_version":1,"status":"%s","complete":true,"exit_code":%s,"test_exit_code":%s,"report_exit_code":%s,"line_exit_code":%s,"branch_exit_code":%s,"profile_exit_code":%s}\n' \
  "$verdict" "$status" "$test_status" "$report_status" "$line_status" "$branch_status" "$profile_status" > "$evidence/run.json"; then
  if [ "$status" -eq 0 ]; then status=1; fi
fi
exit "$status"
