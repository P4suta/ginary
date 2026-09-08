#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Run stable libtest with attributable captured output and durable outcome evidence.

The --list pass names planned tests. A serial --show-output pass keeps reported
tool skips visible and identifies the single interrupted test when libtest has
printed its start. A killed runner's last record remains 'running', never success.
"""
import argparse
import codecs
import json
import os
import pathlib
import re
import signal
import subprocess
import sys


def harness(line, previous):
    match = re.match(r"\s*Running (.+?) \([^)]*\)\s*$", line)
    if match:
        return match[1]
    match = re.match(r"\s*Doc-tests (.+)\s*$", line)
    return "doc:" + match[1] if match else previous


def summarize(listing, execution, status, exit_code):
    return summarize_lines(listing.splitlines(), execution.splitlines(), status, exit_code)


def planned_name(rows, current, displayed):
    """Resolve libtest display-only annotations against names from --list.

    Exact matches win: a test's real name may itself end in the annotation.
    Never remove arbitrary suffixes or conflate tests from different harnesses.
    """
    if (current, displayed) in rows:
        return displayed
    for suffix in (" - should panic", " - compile"):
        if displayed.endswith(suffix):
            name = displayed.removesuffix(suffix)
            if (current, name) in rows:
                return name
    return displayed


def summarize_lines(listing, execution, status, exit_code):
    rows = {}
    current = None
    for line in listing:
        line = line.rstrip("\r\n")
        current = harness(line, current)
        match = re.match(r"(.+): test$", line)
        if current and match:
            name = match[1]
            rows[(current, name)] = {"harness": current, "name": name, "status": "not_run"}
    current = None
    captured = None
    active = None
    for line in execution:
        line = line.rstrip("\r\n")
        updated = harness(line, current)
        if updated != current:
            captured = None
            active = None
        current = updated
        match = re.match(r"test (.+) \.\.\. (ok|FAILED|ignored)(?:, (.*))?$", line)
        if current and match:
            name, result, reason = match.groups()
            name = planned_name(rows, current, name)
            row = rows.setdefault((current, name), {"harness": current, "name": name})
            row["status"] = {"ok": "successful", "FAILED": "failed", "ignored": "skipped"}[result]
            if reason:
                row["reason"] = reason
            captured = None
            active = None
            continue
        if active is not None and line in ("ok", "FAILED", "ignored"):
            active["status"] = {"ok": "successful", "FAILED": "failed", "ignored": "skipped"}[line]
            active = None
            continue
        match = re.match(r"---- (.+) stdout ----$", line)
        if match and current:
            captured = rows.get((current, planned_name(rows, current, match[1])))
        elif captured is not None and line.startswith("skipping: "):
            if captured["status"] == "successful":
                captured.update(status="skipped", reason=line.removeprefix("skipping: "))
        elif line in ("successes:", "failures:") or line.startswith("test result:"):
            captured = None
        match = re.match(r"test (.+) \.\.\.(?: .*)?$", line)
        if current and match and captured is None:
            name = planned_name(rows, current, match[1])
            active = rows.setdefault((current, name), {"harness": current, "name": name, "status": "not_run"})
    if active is not None and status != "running":
        active["status"] = "interrupted"
    tests = [rows[key] for key in sorted(rows)]
    counts = {outcome: sum(row["status"] == outcome for row in tests) for outcome in
              ("successful", "failed", "skipped", "not_run", "interrupted")}
    return {"schema_version": 1, "status": status, "exit_code": exit_code,
            "complete": status in ("successful", "failed") and not counts["not_run"] and not counts["interrupted"],
            "counts": counts, "tests": tests}


def write_report(path, report):
    staged = path.with_suffix(".tmp")
    with staged.open("w", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    staged.replace(path)


def run(command, path):
    child = None
    interrupted = False
    with path.open("w", encoding="utf-8") as log:
        decoder = codecs.getincrementaldecoder("utf-8")("replace")

        def emit(chunk, final=False):
            text = decoder.decode(chunk, final=final)
            log.write(text)
            log.flush()
            sys.stdout.write(text)
            sys.stdout.flush()

        try:
            child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     start_new_session=os.name != "nt")
            while chunk := child.stdout.read1(65536):
                emit(chunk)
            emit(b"", final=True)
            code = child.wait()
        except KeyboardInterrupt:
            interrupted = True
            if child is not None:
                if os.name == "nt":
                    child.kill()
                else:
                    os.killpg(child.pid, signal.SIGKILL)
                while chunk := child.stdout.read1(65536):
                    emit(chunk)
                emit(b"", final=True)
                child.wait()
            code = 130
        except OSError as error:
            message = f"test-evidence: could not execute {command[0]}: {error}\n"
            log.write(message)
            sys.stderr.write(message)
            code = 127
    return code, interrupted


def log_lines(path):
    if path is not None and path.exists():
        with path.open(encoding="utf-8") as stream:
            # Log files retain every byte; parsing needs only bounded line chunks.
            yield from iter(lambda: stream.readline(65536), "")


def snapshot(listing, execution, status, exit_code):
    return summarize_lines(log_lines(listing), log_lines(execution), status, exit_code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    command = options.command
    if command and command[0] == "--":
        command = command[1:]
    if not command or "--" in command:
        parser.error("provide cargo test arguments without a libtest -- separator")
    options.output.mkdir(parents=True, exist_ok=True)
    report = options.output / "outcomes.json"
    os.environ["CARGO_TERM_COLOR"] = "never"
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt()

    signal.signal(signal.SIGTERM, interrupted)
    write_report(report, summarize("", "", "running", None))
    listing = options.output / "planned.log"
    execution = options.output / "tests.log"
    code, stopped = run(command + ["--", "--list", "--format=terse"], listing)
    if code:
        write_report(report, snapshot(listing, None, "interrupted" if stopped else "not_run", code))
        return code
    write_report(report, snapshot(listing, None, "running", None))
    code, stopped = run(command + ["--", "--show-output", "--test-threads=1"], execution)
    result = snapshot(listing, execution, "interrupted" if stopped else "failed" if code else "successful", code)
    if code == 0 and not result["complete"]:
        code = 1
        result.update(status="failed", exit_code=code, reason="test outcome evidence was incomplete")
    write_report(report, result)
    return code


if __name__ == "__main__":
    sys.exit(main())
