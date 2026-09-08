#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Keep every cargo-mutants candidate accountable across native runner jobs.

Discovery is deliberately independent of the host cfg: cargo-mutants 27.1.0
lists inactive code too. The Rust AST tool assigns a native OS, then this adapter
re-lists the exact named subset before execution. Never shard a regex-filtered
list again: cargo-mutants applies its regex before its sharding algorithm.
Full child output stays in files; diagnostics include the last MiB per stream.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import time

RUNNERS = {"linux": "ubuntu-24.04", "windows": "windows-2025", "macos": "macos-15"}
VERSION = "27.1.0"
TAIL_LIMIT = 1024 * 1024
JSON_LIMIT = 128 * 1024 * 1024


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


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def contained(root, relative):
    root = Path(root).resolve()
    if not isinstance(relative, str) or not relative or "\\" in relative:
        raise ValueError(f"invalid evidence path: {relative!r}")
    path = (root / relative).resolve()
    if Path(relative).is_absolute() or path == root or root not in path.parents:
        raise ValueError(f"evidence path escapes root: {relative!r}")
    return path


def names(candidates):
    if not isinstance(candidates, list):
        raise ValueError("candidate list must be an array")
    result = [candidate["name"] for candidate in candidates]
    if any(not isinstance(name, str) or not name for name in result):
        raise ValueError("candidate has no nonempty name")
    if len(set(result)) != len(result):
        raise ValueError("duplicate candidate ID")
    return result


def exact_regex(ids):
    if not ids or any(not isinstance(name, str) or not name for name in ids) or len(set(ids)) != len(ids):
        raise ValueError("selection must be nonempty and unique")
    # Rust regex does not accept Python's escapes for spaces and punctuation.
    escape = lambda value: re.sub(r"([\\.^$|?*+(){}\[\]])", r"\\\1", value)
    return "^(?:" + "|".join(escape(name) for name in ids) + ")$"


def tail(path):
    size = path.stat().st_size
    with path.open("rb") as stream:
        stream.seek(max(0, size - TAIL_LIMIT))
        text = stream.read(TAIL_LIMIT).decode("utf-8", errors="replace")
    return {"bytes": size, "omitted_bytes": max(0, size - TAIL_LIMIT), "tail": text}


def kill_tree(child):
    cleanup_error = None
    if os.name == "nt":
        try:
            result = subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30, check=False)
            if result.returncode:
                cleanup_error = f"taskkill exited {result.returncode}; descendant termination is unconfirmed"
        except (OSError, subprocess.TimeoutExpired) as error:
            cleanup_error = f"taskkill failed; descendant termination is unconfirmed: {error}"
        if child.poll() is None:
            child.kill()
    else:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    child.wait(timeout=30)
    if cleanup_error:
        raise OSError(cleanup_error)


def execute(command, output, timeout, cwd=None, env=None):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    record = {"schema_version": 1, "command": command, "status": "running",
              "exit_code": None, "timeout_seconds": timeout}
    write(output / "process.json", record)
    started = time.monotonic()
    child = None
    interruption = False
    with (output / "stdout.log").open("wb") as stdout, (output / "stderr.log").open("wb") as stderr:
        try:
            child = subprocess.Popen(command, stdout=stdout, stderr=stderr, cwd=cwd, env=env,
                                     start_new_session=os.name != "nt")
            record["exit_code"] = child.wait(timeout=timeout)
            record["status"] = "successful" if record["exit_code"] == 0 else "failed"
        except subprocess.TimeoutExpired:
            record.update(status="timeout", reason="child exceeded execution budget")
        except KeyboardInterrupt:
            interruption = True
            record.update(status="interrupted", reason="runner interrupted")
        except OSError as error:
            record.update(status="spawn_failed", reason=str(error))
        finally:
            if child is not None and child.poll() is None:
                try:
                    kill_tree(child)
                except (OSError, subprocess.TimeoutExpired) as error:
                    record["cleanup_error"] = str(error)
            if child is not None:
                record["exit_code"] = child.poll()
    record.update(elapsed_seconds=time.monotonic() - started,
                  stdout=tail(output / "stdout.log"), stderr=tail(output / "stderr.log"))
    write(output / "process.json", record)
    if interruption:
        raise KeyboardInterrupt()
    return record


def successful(command, output, timeout=120, cwd=None):
    result = execute(command, output, timeout, cwd)
    if result["status"] != "successful":
        raise ValueError(f"command {command[:3]}: {result['status']}; see {output}/process.json")
    return result


def source_manifest(root):
    root = Path(root)
    files = []
    # Inputs used by the tests as well as the crate and the gate itself.
    excluded = {"target", ".git", ".cache", "build", "__pycache__", "mutants.out"}
    for directory in ("src", "tests", "examples", "scripts", "tools/mutation-plan",
                      ".github", ".cargo", ".config", "formal", "docs", "fuzz/seeds"):
        for path in (root / directory).rglob("*"):
            if path.is_file() and not any(part in excluded for part in path.relative_to(root).parts):
                files.append(path)
    files.extend(path for path in root.iterdir() if path.is_file() and path.suffix != ".log")
    return {path.relative_to(root).as_posix(): digest(path) for path in sorted(set(files))}


def verify_sources(root, manifest):
    actual = source_manifest(root)
    if actual != manifest:
        different = sorted(key for key in actual.keys() | manifest.keys() if actual.get(key) != manifest.get(key))
        raise ValueError(f"source inputs differ from planned checkout: {different}")


def make_bundle(canonical, enriched, sources, divisions):
    if (not canonical or not divisions.get("modules") or divisions.get("schema_version") != 1
            or any(type(count) is not int or count < 1 for count in divisions["modules"].values())
            or type(divisions.get("max_mutants_per_shard")) is not int
            or not 1 <= divisions["max_mutants_per_shard"] <= 13
            or any(type(divisions.get(key)) is not int or divisions[key] <= 0
                   for key in ("build_timeout_seconds", "test_timeout_seconds"))):
        raise ValueError("empty or invalid canonical mutation divisions")
    originals = [candidate for shard in canonical for candidate in shard["candidates"]]
    original_ids, enriched_ids = names(originals), names(enriched)
    if not originals:
        raise ValueError("mutation plan has no candidates")
    if set(original_ids) != set(enriched_ids):
        raise ValueError("AST assignment added or dropped candidates")
    lookup = {candidate["name"]: candidate for candidate in enriched}
    jobs = []
    for shard in canonical:
        # cargo-mutants' ceiling division can leave a final canonical shard
        # empty (121 candidates over 12 shards gives eleven 11s and a zero).
        # Preserve that discovery row; it needs no native execution job.
        if len(shard["candidates"]) > divisions["max_mutants_per_shard"]:
            raise ValueError(f"canonical shard {shard['id']} exceeds budget")
        groups = {}
        for original in shard["candidates"]:
            candidate = lookup[original["name"]]
            if {key: value for key, value in candidate.items() if key != "applicability"} != original:
                raise ValueError("AST assignment changed original candidate fields")
            assignment = candidate["applicability"]
            supported = assignment["platforms"]
            assigned = assignment["assigned_platform"]
            if (not isinstance(supported, list) or not supported or len(set(supported)) != len(supported)
                    or any(item not in RUNNERS for item in supported) or assigned not in supported):
                raise ValueError(f"invalid native assignment for {candidate['name']}")
            groups.setdefault(assigned, []).append(candidate["name"])
        for assigned, ids in sorted(groups.items()):
            jobs.append({"id": shard["id"] + "-" + assigned, "canonical_id": shard["id"],
                         "module": shard["module"], "platform": assigned, "runner": RUNNERS[assigned],
                         "candidates": ids})
    if len(jobs) > 256:
        raise ValueError("native matrix exceeds GitHub's 256-job limit")
    return {"schema_version": 1, "cargo_mutants_version": VERSION, "features": ["cli", "fault-injection"],
            "divisions": divisions, "sources": sources, "canonical": canonical,
            "candidates": enriched, "jobs": jobs}


def validate_bundle(bundle):
    if bundle["schema_version"] != 1 or bundle["cargo_mutants_version"] != VERSION:
        raise ValueError("unsupported mutation plan version")
    rebuilt = make_bundle(bundle["canonical"], bundle["candidates"], bundle["sources"], bundle["divisions"])
    if rebuilt != bundle:
        raise ValueError("mutation plan does not match its canonical candidates")
    divisions = bundle["divisions"]
    expected = {f"{module}-{index}": (module, f"{index}/{count}")
                for module, count in divisions["modules"].items() for index in range(count)}
    actual = {shard["id"]: (shard["module"], shard["shard"]) for shard in bundle["canonical"]}
    if len(actual) != len(bundle["canonical"]) or actual != expected:
        raise ValueError("canonical shard set is incomplete or duplicated")
    if bundle["features"] != ["cli", "fault-injection"]:
        raise ValueError("AST cfg assumptions do not match enabled features")


def list_command(module, *selection):
    return ["cargo", "mutants", "--list", "--json", "--diff", "--file", f"src/{module}.rs",
            "--features", "fault-injection", *selection]


def run_command(job, limits, output):
    return ["cargo", "mutants", "--file", f"src/{job['module']}.rs", "--re", exact_regex(job["candidates"]),
            "--timeout", str(limits["test_timeout_seconds"]),
            "--build-timeout", str(limits["build_timeout_seconds"]), "--features", "fault-injection",
            "--output", str(output), "--cargo-test-arg=--", "--cargo-test-arg=--show-output"]


def mutation_environment(output):
    environment = dict(os.environ, GINARY_TEST_EVIDENCE_DIR=str(output / "test-failures"))
    # A cached test binary embeds CARGO_MANIFEST_DIR. Reusing a target from
    # another cargo-mutants scratch copy can retain paths to a deleted tree.
    # Let cargo-mutants own its fresh build directory (copy_target defaults off).
    environment.pop("CARGO_TARGET_DIR", None)
    environment.pop("CARGO_BUILD_TARGET_DIR", None)
    return environment


def version_check(output):
    successful(["cargo", "mutants", "--version"], output)
    version = (output / "stdout.log").read_text(encoding="utf-8").strip()
    if version != f"cargo-mutants {VERSION}":
        raise ValueError(f"unexpected cargo-mutants version: {version}")


def plan(options):
    root, output = options.source_root.resolve(), options.output.resolve()
    version_check(output / "version")
    divisions = read(root / "scripts/ci/mutation-divisions.json")
    snapshot = source_manifest(root)
    canonical = []
    for module, count in divisions["modules"].items():
        for index in range(count):
            shard_id = f"{module}-{index}"
            shard = f"{index}/{count}"
            destination = output / "discovery" / shard_id
            successful(list_command(module, "--shard", shard), destination, cwd=root)
            candidates = read(destination / "stdout.log")
            canonical.append({"id": shard_id, "module": module, "shard": shard, "candidates": candidates})
    original = output / "original-candidates.json"
    write(original, [candidate for shard in canonical for candidate in shard["candidates"]])
    enriched = output / "assigned-candidates.json"
    successful([str(options.planner.resolve()), "--source-root", str(root), "--input", str(original),
                "--output", str(enriched)], output / "cfg-assignment")
    bundle = make_bundle(canonical, read(enriched), snapshot, divisions)
    validate_bundle(bundle)
    verify_sources(root, snapshot)
    write(output / "plan.json", bundle)
    matrix = {"include": [{key: job[key] for key in ("id", "module", "platform", "runner")}
                          for job in bundle["jobs"]]}
    write(output / "matrix.json", matrix)
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write("matrix=" + json.dumps(matrix, separators=(",", ":")) + "\n")
    print(f"Planned {len(bundle['candidates'])} candidates in {len(canonical)} canonical / {len(bundle['jobs'])} native shards")


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


def outcome_report(bundle, job, directory, process_status, process_exit):
    """Missing/partial files are evidence of incomplete work, never empty success."""
    expected = set(job["candidates"])
    rows = {name: {"name": name, "status": "not_run"} for name in sorted(expected)}
    errors, baseline, evidence = [], [], {}
    directory = Path(directory)
    path = directory / "mutants.out/outcomes.json"
    candidate_lookup = {candidate["name"]: candidate for candidate in bundle["candidates"]}
    try:
        document = read(path)
        raw = document["outcomes"]
        if not isinstance(raw, list):
            raise ValueError("outcomes is not an array")
        evidence["mutants.out/outcomes.json"] = digest(path)
        seen = set()
        for result in raw:
            if not valid_phases(result):
                raise ValueError(f"summary contradicts actual phases: {result['summary']}")
            scenario = result["scenario"]
            if scenario == "Baseline":
                if result["diff_path"] is not None:
                    raise ValueError("baseline must not contain a mutation diff")
                baseline.append(result["summary"])
            else:
                mutant = scenario["Mutant"]
                name = mutant["name"]
                if name not in expected or name in seen:
                    raise ValueError(f"unknown or duplicate outcome: {name}")
                seen.add(name)
                original = candidate_lookup[name]
                for key in ("name", "package", "file", "function", "span", "replacement", "genre"):
                    if mutant.get(key) != original.get(key):
                        raise ValueError(f"outcome candidate fields differ: {name}: {key}")
                status = {"CaughtMutant": "caught", "Unviable": "unviable", "MissedMutant": "missed",
                          "Timeout": "timeout"}.get(result["summary"])
                if status is None:
                    raise ValueError(f"unknown mutant summary: {result['summary']}")
                rows[name].update(status=status, phases=result["phase_results"])
            for key in ("log_path", "diff_path"):
                relative = result[key]
                if relative is None and key == "diff_path" and scenario == "Baseline":
                    continue
                evidence_path = "mutants.out/" + relative
                file = contained(directory, evidence_path)
                if not file.is_file() or file.stat().st_size == 0:
                    raise ValueError(f"missing or empty {key}: {relative}")
                if key == "diff_path" and file.read_text(encoding="utf-8") != original["diff"]:
                    raise ValueError(f"mutation diff does not match canonical candidate: {relative}")
                evidence[evidence_path] = digest(file)
    except (OSError, ValueError, KeyError, TypeError) as error:
        errors.append(str(error))
    counts = {status: sum(row["status"] == status for row in rows.values())
              for status in ("caught", "unviable", "missed", "timeout", "not_run")}
    complete = baseline == ["Success"] and not counts["not_run"] and not errors
    passed = complete and not counts["missed"] and not counts["timeout"] and process_status == "successful" and process_exit == 0
    return {"schema_version": 1, "job": job["id"], "platform": job["platform"],
            "process_status": process_status, "process_exit_code": process_exit,
            "baseline": baseline, "complete": complete, "mutation_gate_passed": passed,
            "counts": counts, "errors": errors, "candidates": list(rows.values()), "evidence": evidence}


def native_run(options):
    root, output = options.source_root.resolve(), options.output.resolve()
    bundle = read(options.bundle)
    validate_bundle(bundle)
    matching = [job for job in bundle["jobs"] if job["id"] == options.job]
    if len(matching) != 1:
        raise ValueError(f"unknown job: {options.job}")
    job = matching[0]
    report_path = output / "report.json"
    plan_sha = digest(options.bundle)
    status, code, failure = "not_run", None, None
    write(report_path, {"schema_version": 1, "job": job["id"], "plan_sha256": plan_sha,
                        "status": "not_run", "complete": False, "mutation_gate_passed": False})
    try:
        native = {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}.get(platform.system())
        if native != job["platform"]:
            raise ValueError(f"job needs {job['platform']}, executing on {native}")
        verify_sources(root, bundle["sources"])
        version_check(output / "version")
        # A native job promises these tools and tests their real startup.
        # Windows/macOS do not promise Linux's Docker daemon or /bin/sh.
        successful(["gleam", "--version"], output / "tools/gleam")
        erl = "erl.exe" if os.name == "nt" else "erl"
        successful([erl, "-noshell", "-eval", 'io:format("~s", [erlang:system_info(otp_release)]), halt(0).'],
                   output / "tools/erl")
        selection = exact_regex(job["candidates"])
        command = list_command(job["module"], "--re", selection)
        successful(command, output / "selection", cwd=root)
        selected = read(output / "selection/stdout.log")
        originals = {candidate["name"]: {key: value for key, value in candidate.items() if key != "applicability"}
                     for candidate in bundle["candidates"] if candidate["name"] in job["candidates"]}
        if {candidate["name"]: candidate for candidate in selected} != originals or set(names(selected)) != set(job["candidates"]):
            raise ValueError("native discovery differs from assigned candidates")
        limits = bundle["divisions"]
        command = run_command(job, limits, output)
        status = "running"
        environment = mutation_environment(output)
        write(output / "selection.json", {"job": job, "command": command, "plan_sha256": plan_sha,
              "require_toolchain": os.environ.get("GINARY_REQUIRE_TOOLCHAIN") == "1",
              "test_evidence_dir": environment["GINARY_TEST_EVIDENCE_DIR"],
              "build_target_policy": "cargo-mutants scratch; inherited target directory overrides removed",
              "required_tools": ["gleam", "erl"], "optional_tools": "reported as skipping: in full test logs"})
        result = execute(command, output / "execution",
                         (len(job["candidates"]) + 1) * (limits["test_timeout_seconds"] + limits["build_timeout_seconds"]) + 120,
                         cwd=root, env=environment)
        status, code = result["status"], result["exit_code"]
        verify_sources(root, bundle["sources"])
    except KeyboardInterrupt:
        status, failure = "interrupted", "runner interrupted"
    except (OSError, ValueError, KeyError, TypeError) as error:
        status, failure = "failed", str(error)
    finally:
        report = outcome_report(bundle, job, output, status, code)
        report["plan_sha256"] = plan_sha
        if failure:
            report["errors"].append(failure)
            report["mutation_gate_passed"] = False
        write(report_path, report)
        print(json.dumps({key: value for key, value in report.items() if key not in ("candidates", "evidence")}, indent=2))
    return 0 if report["mutation_gate_passed"] else 1


def reconcile(bundle_path, evidence_root):
    bundle = read(bundle_path)
    validate_bundle(bundle)
    plan_sha = digest(bundle_path)
    errors, reports = [], []
    qualified_jobs = 0
    evidence_root = Path(evidence_root)
    expected_dirs = {"mutants-" + job["id"] for job in bundle["jobs"]}
    actual_dirs = {path.name for path in evidence_root.iterdir() if path.is_dir()} if evidence_root.exists() else set()
    if actual_dirs != expected_dirs:
        errors.append(f"native evidence directories differ: missing={sorted(expected_dirs - actual_dirs)}, extra={sorted(actual_dirs - expected_dirs)}")
    for job in bundle["jobs"]:
        directory = evidence_root / ("mutants-" + job["id"])
        # Account for every assigned candidate even if its job never started,
        # its process failed, or the saved report is missing/contradictory.
        try:
            execution = read(directory / "execution/process.json")
            if not isinstance(execution, dict):
                raise ValueError("process record is not an object")
        except (OSError, ValueError):
            execution = {"status": "not_run", "exit_code": None}
        recomputed = outcome_report(bundle, job, directory, execution.get("status", "not_run"), execution.get("exit_code"))
        reports.append(recomputed)
        try:
            saved = read(directory / "report.json")
            if saved["job"] != job["id"] or saved["plan_sha256"] != plan_sha:
                raise ValueError("report belongs to another job or plan")
            execution = read(directory / "execution/process.json")
            if (execution["status"] != saved["process_status"]
                    or execution["exit_code"] != saved["process_exit_code"]
                    or execution.get("cleanup_error")):
                raise ValueError("process record differs from reported execution")
            for stream in ("stdout", "stderr"):
                if execution[stream] != tail(directory / "execution" / (stream + ".log")):
                    raise ValueError(f"captured {stream} differs from process record")
            selection = read(directory / "selection.json")
            if selection["job"] != job or selection["plan_sha256"] != plan_sha or selection["command"] != execution["command"]:
                raise ValueError("execution did not use the assigned selection")
            command = execution["command"]
            if (not isinstance(command, list) or "--output" not in command
                    or command.index("--output") + 1 >= len(command)
                    or command != run_command(job, bundle["divisions"], command[command.index("--output") + 1])):
                raise ValueError("execution command does not match the planned budget and candidate selection")
            expected_candidates = {candidate["name"]: {key: value for key, value in candidate.items() if key != "applicability"}
                                   for candidate in bundle["candidates"] if candidate["name"] in job["candidates"]}
            selected_candidates = read(directory / "selection/stdout.log")
            if (set(names(selected_candidates)) != set(job["candidates"])
                    or {candidate["name"]: candidate for candidate in selected_candidates} != expected_candidates):
                raise ValueError("selection discovery added, dropped or changed candidates")
            for key in ("platform", "baseline", "complete", "counts", "candidates", "evidence"):
                if saved.get(key) != recomputed[key]:
                    raise ValueError(f"saved report does not match actual evidence: {key}")
            if not saved["mutation_gate_passed"] or not recomputed["mutation_gate_passed"] or saved["errors"]:
                raise ValueError("mutation gate failed or execution incomplete")
            qualified_jobs += 1
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"{job['id']}: {error}")
    passed = not errors and qualified_jobs == len(bundle["jobs"])
    return {"schema_version": 1, "plan_sha256": plan_sha, "mutation_gate_passed": passed,
            "planned_candidates": len(bundle["candidates"]), "planned_jobs": len(bundle["jobs"]),
            "qualified_jobs": qualified_jobs, "errors": errors,
            "jobs": [{key: report[key] for key in ("job", "platform", "counts", "complete", "errors")}
                     for report in reports],
            "counts": {status: sum(report["counts"][status] for report in reports)
                       for status in ("caught", "unviable", "missed", "timeout", "not_run")}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    planning = subcommands.add_parser("plan")
    planning.add_argument("--source-root", type=Path, default=Path.cwd())
    planning.add_argument("--planner", type=Path, required=True)
    planning.add_argument("--output", type=Path, required=True)
    running = subcommands.add_parser("run")
    running.add_argument("--source-root", type=Path, default=Path.cwd())
    running.add_argument("--bundle", type=Path, required=True)
    running.add_argument("--job", required=True)
    running.add_argument("--output", type=Path, required=True)
    final = subcommands.add_parser("finalize")
    final.add_argument("--bundle", type=Path, required=True)
    final.add_argument("--evidence", type=Path, required=True)
    final.add_argument("--output", type=Path, required=True)
    options = parser.parse_args()
    os.environ["CARGO_TERM_COLOR"] = "never"
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt()
    signal.signal(signal.SIGTERM, interrupted)
    try:
        if options.command == "plan":
            plan(options)
            return 0
        if options.command == "run":
            return native_run(options)
        result = reconcile(options.bundle, options.evidence)
        write(options.output, result)
        print(json.dumps(result, indent=2))
        return 0 if result["mutation_gate_passed"] else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        if options.command == "finalize":
            write(options.output, {"schema_version": 1, "mutation_gate_passed": False,
                                   "complete": False, "errors": [str(error)]})
        print(f"mutation: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
