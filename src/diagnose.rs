// SPDX-License-Identifier: MIT OR Apache-2.0
//! Local diagnostic summaries without executing a supplied artifact.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Maximum bytes inspected from a trace in one collection.
pub const MAX_TRACE_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum bytes retained for one JSON Lines record.
pub const MAX_TRACE_LINE_BYTES: usize = 64 * 1024;
/// Maximum distinct runs retained by one trace summary.
pub const MAX_TRACE_RUNS: usize = 1024;
/// Maximum crash-dump prefix inspected in one collection.
pub const MAX_CRASHDUMP_BYTES: u64 = 16 * 1024 * 1024;

/// A versioned, locally collected diagnostic summary.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnoseReport {
    /// Version of the diagnostic summary schema.
    pub format_version: u32,
    /// Collector version, independent of any supplied artifact.
    pub ginary_version: String,
    /// The platform collecting this evidence.
    pub host_target: String,
    /// Whether every requested evidence source was read completely and parsed.
    pub complete: bool,
    /// Sanitized results of known local environment probes.
    pub environment: Option<EnvironmentSummary>,
    /// Artifact integrity statistics; the artifact is never run or extracted.
    pub artifact: Option<ArtifactSummary>,
    /// Trace statistics, without raw event values.
    pub trace: Option<TraceSummary>,
    /// Crash-dump statistics, without raw terms or process names.
    pub crashdump: Option<CrashdumpSummary>,
    /// Failures, limitations, and the next actions they suggest.
    pub findings: Vec<DiagnosticFinding>,
}

/// An evidence collection issue with no raw input contents or local paths.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticFinding {
    /// Evidence source: artifact, trace, crashdump, or collection.
    pub source: String,
    /// Stable issue classification.
    pub code: String,
    /// Controlled explanation of the finding.
    pub reason: String,
    /// Next action the user can take.
    pub remedy: String,
}

/// Environment readiness without paths, tool output, or environment values.
#[derive(Clone, Debug, Serialize)]
pub struct EnvironmentSummary {
    /// Outcome and remedy for each known external tool.
    pub tools: Vec<ToolHealth>,
    /// Whether local OTP discovery succeeded.
    pub otp_available: bool,
    /// Whether the cache write probe succeeded.
    pub cache_writable: bool,
    /// Whether the cache execution probe succeeded.
    pub cache_executable: bool,
    /// Whether a project manifest was successfully read.
    pub project_present: bool,
    /// Stable classifications of environment findings; raw reasons are omitted.
    pub finding_codes: Vec<String>,
}

/// Tool readiness, excluding version-command output and executable paths.
#[derive(Clone, Debug, Serialize)]
pub struct ToolHealth {
    /// Known program name.
    pub name: String,
    /// Explicit process or parse outcome.
    pub outcome: crate::doctor::ProbeOutcome,
    /// Observed exit code, if any.
    pub exit_code: Option<i32>,
    /// Fixed next-action guidance supplied by doctor.
    pub remedy: String,
}

impl EnvironmentSummary {
    fn gather() -> Self {
        let detailed = crate::doctor::DetailedReport::gather();
        Self {
            tools: detailed
                .tool_probes
                .into_iter()
                .map(|probe| ToolHealth {
                    name: probe.tool.name,
                    outcome: probe.outcome,
                    exit_code: probe.exit_code,
                    remedy: probe.remedy,
                })
                .collect(),
            otp_available: detailed.report.otp.is_some(),
            cache_writable: detailed
                .report
                .cache_probe
                .as_ref()
                .is_some_and(|cache| cache.writable),
            cache_executable: detailed
                .report
                .cache_probe
                .as_ref()
                .is_some_and(|cache| cache.executable),
            project_present: detailed.report.project.is_some(),
            finding_codes: detailed
                .findings
                .into_iter()
                .map(|finding| finding.code)
                .collect(),
        }
    }
}

/// Integrity statistics of a locally read artifact.
#[derive(Clone, Debug, Serialize)]
pub struct ArtifactSummary {
    /// Whether payload and indexed files passed all verifier checks.
    pub verified: bool,
    /// Stage outcomes, including checks prevented from running.
    pub checks: crate::verify::VerificationChecks,
    /// Whether the payload matches its digest; absent if it could not be checked.
    pub payload_ok: Option<bool>,
    /// Indexed files checked; absent unless the contents scan completed.
    pub files_checked: Option<usize>,
    /// Native objects inspected; absent unless the contents scan completed.
    pub native_objects: Option<usize>,
    /// Findings count; absent unless the contents scan completed.
    pub issues: Option<usize>,
}

/// Statistics of the inspected crash-dump prefix.
#[derive(Clone, Debug, Serialize)]
pub struct CrashdumpSummary {
    /// Process sections observed.
    pub processes: usize,
    /// Largest observed process heap, in words.
    pub largest_heap_words: u64,
    /// Whether the dump itself ended early.
    pub truncated: bool,
    /// Whether collection stopped at its byte bound.
    pub limit_reached: bool,
}

/// Counts for one trace run, identified by a hash instead of its raw ID.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TraceRun {
    /// SHA-256 of the trace's run ID, permitting local correlation without copying it.
    pub fingerprint: String,
    /// Valid events belonging to this run.
    pub events: u64,
    /// Recorded failure events.
    pub failures: u64,
    /// Operations dropped before an explicit outcome.
    pub interrupted: u64,
}

/// A bounded summary of legacy schema 1 and schema 2 JSON Lines traces.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TraceSummary {
    /// Valid supported events observed.
    pub events: u64,
    /// Lines that were malformed, oversized, or used an unsupported schema.
    pub invalid_lines: u64,
    /// Unsupported schema versions encountered.
    pub unsupported_lines: u64,
    /// Oversized lines skipped without allocating their contents.
    pub oversized_lines: u64,
    /// Supported events grouped by schema version.
    pub schema_versions: BTreeMap<u64, u64>,
    /// Events grouped by the controlled event vocabulary.
    pub outcomes: BTreeMap<String, u64>,
    /// Distinct schema 2 runs observed before the run bound.
    pub run_count: usize,
    /// Per-run statistics; raw run IDs and all event values are omitted.
    pub runs: Vec<TraceRun>,
    /// Whether every line was supported and collection reached EOF.
    pub complete: bool,
    /// Whether the byte or run count bound stopped collection.
    pub limit_reached: bool,
    /// Bytes consumed from the file.
    pub bytes_read: u64,
}

impl DiagnoseReport {
    /// Renders the local summary.
    pub fn render_text(&self) -> String {
        let mut text = format!(
            "ginary diagnosis {}\nhost: {}\ncollection: {}\n",
            self.ginary_version,
            self.host_target,
            if self.complete {
                "complete"
            } else {
                "incomplete"
            }
        );
        if let Some(environment) = &self.environment {
            text.push_str(&format!(
                "environment: OTP {}, cache writable {}, cache executable {}\n",
                environment.otp_available, environment.cache_writable, environment.cache_executable
            ));
            for tool in &environment.tools {
                text.push_str(&format!(
                    "  {}: {:?}\n  remedy: {}\n",
                    tool.name, tool.outcome, tool.remedy
                ));
            }
        }
        if let Some(artifact) = &self.artifact {
            use crate::verify::CheckOutcome;
            text.push_str("artifact: ");
            match artifact.checks.contents {
                CheckOutcome::NotRun => text.push_str("deep verification not run\n"),
                CheckOutcome::Incomplete => text.push_str("deep verification incomplete\n"),
                CheckOutcome::Passed | CheckOutcome::Failed => text.push_str(&format!(
                    "{} ({} files checked, {} issues)\n",
                    if artifact.verified {
                        "verified"
                    } else {
                        "verification failed"
                    },
                    artifact.files_checked.unwrap_or_default(),
                    artifact.issues.unwrap_or_default(),
                )),
            }
        }
        if let Some(trace) = &self.trace {
            text.push_str(&format!(
                "trace: {} events, {} runs, {} invalid lines; {}\n",
                trace.events,
                trace.run_count,
                trace.invalid_lines,
                if trace.complete {
                    "complete"
                } else {
                    "incomplete"
                }
            ));
            for (outcome, count) in &trace.outcomes {
                text.push_str(&format!("  {outcome}: {count}\n"));
            }
        }
        if let Some(dump) = &self.crashdump {
            text.push_str(&format!(
                "crashdump: {} processes, largest heap {} words; {}\n",
                dump.processes,
                dump.largest_heap_words,
                if dump.truncated || dump.limit_reached {
                    "incomplete"
                } else {
                    "complete"
                }
            ));
        }
        for finding in &self.findings {
            text.push_str(&format!(
                "{}: {}\n  remedy: {}\n",
                finding.source, finding.reason, finding.remedy
            ));
        }
        text.push_str("Raw arguments, environment, paths, trace values, and crash-dump terms are not included. No supplied artifact was executed.\n");
        text
    }

    fn finding(&mut self, source: &str, code: &str, reason: &str, remedy: &str) {
        self.findings.push(DiagnosticFinding {
            source: source.into(),
            code: code.into(),
            reason: reason.into(),
            remedy: remedy.into(),
        });
    }
}

/// Collects summaries of explicitly supplied local evidence.
pub fn gather(
    artifact: Option<&Path>,
    trace: Option<&Path>,
    crashdump: Option<&Path>,
) -> DiagnoseReport {
    let mut report = summarize(artifact, trace, crashdump);
    report.environment = Some(EnvironmentSummary::gather());
    report
}

/// Summarizes evidence only, without probing tools or writing a cache probe.
///
/// The command uses [`gather`] to additionally inspect the environment. This
/// read-only seam supports offline incident processing and isolated tests.
pub fn summarize(
    artifact: Option<&Path>,
    trace: Option<&Path>,
    crashdump: Option<&Path>,
) -> DiagnoseReport {
    let mut report = DiagnoseReport {
        format_version: 1,
        ginary_version: env!("CARGO_PKG_VERSION").into(),
        host_target: crate::target::Target::host().name(),
        complete: true,
        environment: None,
        artifact: None,
        trace: None,
        crashdump: None,
        findings: Vec::new(),
    };
    if artifact.is_none() && trace.is_none() && crashdump.is_none() {
        report.finding(
            "collection",
            "no_evidence",
            "No artifact, trace, or crash dump was supplied; incident evidence is unavailable.",
            "Supply an artifact, --trace PATH, or --crashdump PATH to collect evidence.",
        );
    }
    if let Some(path) = artifact {
        use crate::verify::{CheckOutcome, VerificationChecks};
        let unavailable = VerificationChecks {
            integrity: CheckOutcome::Incomplete,
            contents: CheckOutcome::NotRun,
        };
        match regular_file(path)
            .map_err(|_| unavailable)
            .and_then(|_| crate::verify::verify(path).map_err(|error| error.checks()))
        {
            Ok(verified) => {
                if !verified.ok() {
                    report.finding("artifact", "verification_failed", "The artifact failed integrity or native compatibility checks.", "Run ginary verify on the original artifact to inspect the individual findings, then rebuild it.");
                }
                let contents_complete = verified.payload.ok();
                if !contents_complete {
                    report.complete = false;
                }
                report.artifact = Some(ArtifactSummary {
                    verified: verified.ok(),
                    checks: verified.checks(),
                    payload_ok: Some(verified.payload.ok()),
                    files_checked: contents_complete.then_some(verified.files_checked),
                    native_objects: contents_complete.then_some(verified.objects.len()),
                    issues: contents_complete.then_some(verified.issues.len()),
                });
            }
            Err(checks) => {
                report.complete = false;
                report.artifact = Some(ArtifactSummary {
                    verified: false,
                    checks,
                    payload_ok: (checks.integrity == CheckOutcome::Passed).then_some(true),
                    files_checked: None,
                    native_objects: None,
                    issues: None,
                });
                if checks.contents == CheckOutcome::Incomplete {
                    report.finding("artifact", "incomplete_artifact_scan", "Payload integrity passed, but the contents scan could not finish.", "Run ginary verify on the original artifact to identify the read or archive error, then rebuild it.");
                } else {
                    report.finding("artifact", "unreadable_artifact", "The artifact could not be opened or parsed; deep verification was not run.", "Check the input file, then run ginary inspect and ginary verify for the local error details.");
                }
            }
        }
    }
    if let Some(path) = trace {
        match summarize_trace(path) {
            Ok(summary) => {
                if !summary.complete {
                    report.complete = false;
                    report.finding("trace", "incomplete_trace", "Some trace evidence was invalid, unsupported, or outside the collection bounds.", "Keep the original trace; inspect invalid lines or provide a smaller trace segment and collect again.");
                }
                if summary.outcomes.get("failure").copied().unwrap_or(0) > 0
                    || summary.outcomes.get("interrupted").copied().unwrap_or(0) > 0
                {
                    report.finding("trace", "failed_operations", "The trace contains failed or interrupted operations.", "Inspect the original trace near failure and interrupted events, using run identity and sequence to correlate them.");
                }
                report.trace = Some(summary);
            }
            Err(()) => {
                report.complete = false;
                report.finding(
                    "trace",
                    "unreadable_trace",
                    "The trace could not be opened or read as a regular file.",
                    "Check the trace path and read permissions, then collect again.",
                );
            }
        }
    }
    if let Some(path) = crashdump {
        let result = regular_file(path).and_then(|size| {
            let file = std::fs::File::open(path).map_err(|_| ())?;
            let dump = crate::crashdump::parse(BufReader::new(file.take(MAX_CRASHDUMP_BYTES)))
                .map_err(|_| ())?;
            Ok(CrashdumpSummary {
                processes: dump.processes,
                largest_heap_words: dump.top_processes.first().map_or(0, |process| process.heap),
                truncated: dump.truncated,
                limit_reached: size > MAX_CRASHDUMP_BYTES,
            })
        });
        match result {
            Ok(summary) => {
                if summary.truncated || summary.limit_reached {
                    report.complete = false;
                    report.finding("crashdump", "incomplete_crashdump", "The crash dump is truncated or exceeds the inspected prefix.", "Keep the original dump and use ginary crashdump to scan the full file locally.");
                }
                report.crashdump = Some(summary);
            }
            Err(()) => {
                report.complete = false;
                report.finding(
                    "crashdump",
                    "unreadable_crashdump",
                    "The crash dump could not be opened or recognized.",
                    "Check the input path, then run ginary crashdump for local parsing details.",
                );
            }
        }
    }
    report
}

fn regular_file(path: &Path) -> Result<u64, ()> {
    let metadata = std::fs::metadata(path).map_err(|_| ())?;
    if metadata.is_file() {
        Ok(metadata.len())
    } else {
        Err(())
    }
}

fn summarize_trace(path: &Path) -> Result<TraceSummary, ()> {
    let size = regular_file(path)?;
    let file = std::fs::File::open(path).map_err(|_| ())?;
    let mut reader = BufReader::new(file.take(MAX_TRACE_BYTES));
    let mut summary = TraceSummary {
        complete: size <= MAX_TRACE_BYTES,
        limit_reached: size > MAX_TRACE_BYTES,
        ..TraceSummary::default()
    };
    let mut runs = BTreeMap::<String, TraceRun>::new();
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        let mut consumed = 0;
        loop {
            let chunk = reader.fill_buf().map_err(|_| ())?;
            if chunk.is_empty() {
                break;
            }
            let count = chunk
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(chunk.len(), |index| index + 1);
            let ended = chunk[count - 1] == b'\n';
            if line.len().saturating_add(count) <= MAX_TRACE_LINE_BYTES {
                line.extend_from_slice(&chunk[..count]);
            } else {
                oversized = true;
            }
            consumed += count as u64;
            reader.consume(count);
            if ended {
                break;
            }
        }
        if consumed == 0 {
            break;
        }
        summary.bytes_read += consumed;
        if oversized {
            summary.oversized_lines += 1;
            summary.invalid_lines += 1;
            summary.complete = false;
            continue;
        }
        let value = serde_json::from_slice::<serde_json::Value>(&line).ok();
        let Some(value) = value.filter(|value| value.is_object()) else {
            summary.invalid_lines += 1;
            summary.complete = false;
            continue;
        };
        let schema = value
            .get("schema_version")
            .map_or(Some(1), serde_json::Value::as_u64);
        let Some(schema @ (1 | 2)) = schema else {
            summary.unsupported_lines += 1;
            summary.invalid_lines += 1;
            summary.complete = false;
            continue;
        };
        let event = if schema == 1 {
            Some("legacy")
        } else {
            value["event"].as_str().filter(|event| {
                matches!(
                    *event,
                    "start" | "begin" | "end" | "fact" | "failure" | "interrupted"
                )
            })
        };
        let run_id = value["run_id"].as_str().filter(|id| !id.is_empty());
        if !value["t_us"].is_u64()
            || !value["phase"].is_string()
            || !value["kv"]
                .as_object()
                .is_some_and(|kv| kv.values().all(serde_json::Value::is_string))
            || event.is_none()
            || (schema == 2 && (run_id.is_none() || !value["sequence"].is_u64()))
        {
            summary.invalid_lines += 1;
            summary.complete = false;
            continue;
        }
        if schema == 2 {
            let fingerprint = hex::encode(Sha256::digest(run_id.unwrap_or_default().as_bytes()));
            if !runs.contains_key(&fingerprint) && runs.len() >= MAX_TRACE_RUNS {
                summary.limit_reached = true;
                summary.complete = false;
                break;
            }
            let run = runs.entry(fingerprint.clone()).or_insert_with(|| TraceRun {
                fingerprint,
                ..TraceRun::default()
            });
            run.events += 1;
            run.failures += u64::from(event == Some("failure"));
            run.interrupted += u64::from(event == Some("interrupted"));
        }
        summary.events += 1;
        *summary.schema_versions.entry(schema).or_default() += 1;
        *summary
            .outcomes
            .entry(event.unwrap_or("legacy").into())
            .or_default() += 1;
    }
    summary.run_count = runs.len();
    summary.runs = runs.into_values().collect();
    Ok(summary)
}
