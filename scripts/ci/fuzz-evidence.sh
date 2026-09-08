#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Source immediately before cargo fuzz: preserve its exact exit and both streams.
set -euo pipefail
: "${FUZZ_TARGET:?the fuzz target is required}"
fuzz_evidence="${GITHUB_WORKSPACE:-$PWD}/target/fuzz-evidence"
mkdir -p "$fuzz_evidence"
printf '{"schema_version":1,"status":"running","complete":false}\n' > "$fuzz_evidence/run.json"
fuzz_started=$SECONDS
exec 3>&1 4>&2
exec > >(tee "$fuzz_evidence/run.log") 2>&1
fuzz_logger=$!
finish_fuzz_evidence() {
  local command_status=$? log_status status verdict
  trap - EXIT
  set +e
  exec 1>&3 2>&4 3>&- 4>&-
  wait "$fuzz_logger"
  log_status=$?
  status=$command_status
  if [ "$status" -eq 0 ]; then status=$log_status; fi
  verdict=failed
  if [ "$status" -eq 0 ]; then verdict=successful; fi
  if [ "$status" -eq 130 ] || [ "$status" -eq 143 ]; then verdict=interrupted; fi
  if ! printf '{"schema_version":1,"target":"%s","status":"%s","complete":true,"exit_code":%s,"command_exit_code":%s,"log_exit_code":%s,"elapsed_seconds":%s}\n' \
    "$FUZZ_TARGET" "$verdict" "$status" "$command_status" "$log_status" "$((SECONDS - fuzz_started))" > "$fuzz_evidence/run.json"; then
    if [ "$status" -eq 0 ]; then status=1; fi
  fi
  exit "$status"
}
trap finish_fuzz_evidence EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
