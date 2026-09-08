#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Keep the cross-runtime matrix verdict, logs and artifacts after failure.
set -euo pipefail
evidence="${GITHUB_WORKSPACE:-$PWD}/target/assurance/smoke-matrix"
mkdir -p "$evidence"
printf '{"schema_version":1,"status":"running","complete":false}\n' > "$evidence/run.json"
export GINARY_SMOKE_EVIDENCE_DIR="$evidence"
export GINARY_TRACE="$evidence/trace.ndjson"
smoke_started=$SECONDS
set +e
(
  trap 'exit 130' INT
  trap 'exit 143' TERM
  bash scripts/smoke-matrix.sh
) 2>&1 | tee "$evidence/smoke.log"
codes=("${PIPESTATUS[@]}")
command_status=${codes[0]}
log_status=${codes[1]}
status=$command_status
if [ "$status" -eq 0 ]; then status=$log_status; fi
verdict=failed
if [ "$status" -eq 0 ]; then verdict=successful; fi
if [ "$status" -eq 130 ] || [ "$status" -eq 143 ]; then verdict=interrupted; fi
if ! printf '{"schema_version":1,"status":"%s","complete":true,"exit_code":%s,"command_exit_code":%s,"log_exit_code":%s,"elapsed_seconds":%s}\n' \
  "$verdict" "$status" "$command_status" "$log_status" "$((SECONDS - smoke_started))" > "$evidence/run.json"; then
  if [ "$status" -eq 0 ]; then status=1; fi
fi
exit "$status"
