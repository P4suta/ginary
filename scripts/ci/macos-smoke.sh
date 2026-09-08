#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Native macOS packaging/signature rehearsal with retained diagnostic evidence.
# Kept in a file so ShellCheck reads a path instead of actionlint forwarding
# an oversized stdin pipe before starting its child process on Windows.
set -euo pipefail
: "${GITHUB_WORKSPACE:?the workspace root is required}"
: "${GINARY_SMOKE_TARGET:?the native macOS target is required}"
evidence="$GITHUB_WORKSPACE/target/assurance/${GINARY_SMOKE_TARGET}"
mkdir -p "$evidence"
printf '{"status":"running","complete":false}\n' > "$evidence/run.json"
# Keep the exact command verdict even when tee or diagnostics fail.
set +e
(
set -euo pipefail
trap 'status=$?; printf "{\"exit_code\":%s}\n" "$status" > "$evidence/command.json"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export GINARY_TRACE="$evidence/trace.ndjson"
export GINARY_CACHE_DIR="$evidence/cache"
otp_root="$(dirname "$(dirname "$(command -v erl)")")"
work=$(mktemp -d "$evidence/work.XXXXXX")
cp -R tests/fixtures/hello_ffi "$work/hello_ffi"
printf '\n[tools.ginary.target."%s"]\nerts = "dir:%s"\n' \
  "${GINARY_SMOKE_TARGET}" "$otp_root" >> "$work/hello_ffi/gleam.toml"
( cd "$work/hello_ffi"
  "$GITHUB_WORKSPACE/target/release/ginary" build \
    --target "${GINARY_SMOKE_TARGET}" )
artifact="$work/hello_ffi/build/ginary/hello_ffi-${GINARY_SMOKE_TARGET}"
# Diagnostic: the stub's load commands beside the finished artifact's.
# A valid signature over a structurally broken image verifies and then
# segfaults on exec (exit 139), which is what both runners reported on
# run 33724862229: the E8 writer injected a new segment, and making
# room for its load command slid the code forward — invalidating the
# entry point and every LC_DYLD_CHAINED_FIXUPS rebase target, which the
# signature happily covered. E9 rewrote the writer to grow __LINKEDIT
# over the payload instead, moving nothing. These dumps are the
# evidence: __TEXT's fileoff/filesize, LC_MAIN's entryoff and every
# segment before __LINKEDIT must read identically in both columns, and
# only __LINKEDIT's filesize/vmsize and LC_CODE_SIGNATURE's
# dataoff/datasize may differ. They cost a second on the third round
# of this bug and stay for the fourth. See docs/dev/log/E9.md.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
stub="$GITHUB_WORKSPACE/target/stubs/ginary-stub-${version}-${GINARY_SMOKE_TARGET}"
fields='cmd |segname|fileoff|filesize |vmaddr|vmsize|entryoff|dataoff|datasize'
echo "== stub load commands =="
otool -l "$stub" | grep -E "$fields" || true
echo "== artifact load commands =="
otool -l "$artifact" | grep -E "$fields" || true
# The signature is read and checked *before* the artifact is run, and
# not only after it. An invalid ad-hoc signature is not a program
# that starts and misbehaves: the kernel refuses to map a page whose
# hash disagrees with the CodeDirectory and kills the process with
# SIGKILL before `main`. Checking first turns that back into a sentence
# naming the defect, and `--display` prints the geometry a future one
# needs: the CDHash, the flags, the hash type and the executable
# segment. See docs/dev/log/E8.md and docs/dev/log/E9.md.
codesign --display --verbose=4 "$artifact" || true
codesign --verify --strict --verbose=4 "$artifact"
# Run the artifact, and on any non-zero exit print the crash report the
# runner keeps before failing: a low faulting address is a null deref
# in our own first instructions, a fault inside dyld is a malformed
# image. The exit code is still asserted — the diagnostics do not
# soften it.
set +e
"$artifact" 0 hello world
status=$?
set -e
if [ "$status" -ne 0 ]; then
  echo "::error::the hello_ffi artifact exited $status, not 0"
  find "$HOME/Library/Logs/DiagnosticReports" -maxdepth 1 -type f \
    -name '*.ips' 2>/dev/null || true
  mkdir -p "$evidence/crashes"
  for report in "$HOME"/Library/Logs/DiagnosticReports/hello_ffi*.ips; do
    if [ -f "$report" ]; then
      cp "$report" "$evidence/crashes/" || true
      echo "== $report =="
      cat "$report" || true
    fi
  done
  log show --last 5m \
    --predicate "process == \"hello_ffi-${GINARY_SMOKE_TARGET}\"" 2>/dev/null \
    | tail -n 40 || true
  exit "$status"
fi
# Assert exit 3; the regression executes this block and the wrapper.
set +e
"$artifact" 3
status=$?
set -e
printf '{"expected":3,"observed":%s}\n' "$status" > "$evidence/exit-code.json"
if [ "$status" -ne 3 ]; then
  echo "::error::the hello_ffi artifact exited $status, expected 3" >&2
  exit 1
fi
# And again afterwards: the artifact extracts its payload into a
# cache directory and must not have touched its own file to do it.
codesign --verify --strict --verbose=4 "$artifact"
) 2>&1 | tee "$evidence/smoke.log"
statuses=("${PIPESTATUS[@]}")
set -e
command_status=${statuses[0]}
log_status=${statuses[1]}
status=$command_status
if [ "$status" -eq 0 ] && [ "$log_status" -ne 0 ]; then status=$log_status; fi
verdict=failed
if [ "$status" -eq 0 ]; then verdict=successful; fi
if [ "$status" -eq 130 ] || [ "$status" -eq 143 ]; then verdict=interrupted; fi
printf '{"status":"%s","complete":true,"exit_code":%s,"command_exit_code":%s,"log_exit_code":%s}\n' \
  "$verdict" "$status" "$command_status" "$log_status" > "$evidence/run.json"
exit "$status"
