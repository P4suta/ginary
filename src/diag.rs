// SPDX-License-Identifier: MIT OR Apache-2.0
//! A versioned event recorder for the launcher and `ginary build -v`.
//!
//! The launcher has to be explainable without being slow. Two environment
//! variables turn it on and nothing else does:
//!
//! - `GINARY_DEBUG=1` writes human lines to standard error,
//!   `ginary[debug]: <phase> <k=v ...> (<elapsed_us>us)`;
//! - `GINARY_TRACE=<path>` appends JSON Lines to that file,
//!   one version-2 object per line, with run identity, event sequence,
//!   elapsed time, phase and redacted facts. [`Diag::operation`] adds paired
//!   lifecycle events with an operation identifier.
//!
//! `GINARY_TRACE_SENSITIVE=1` explicitly opts into sensitive argument and
//! environment facts; URL credentials remain redacted in either mode.
//!
//! With neither set every method is a no-op and no clock is read at all:
//! [`Diag::disabled`] is what the launcher holds, and a [`PhaseGuard`] it
//! hands out carries no [`Instant`] to subtract on drop. Timing that only the
//! debugging path pays for is timing the fast path does not.
//!
//! Both sinks are injected rather than opened, which is how the tests read
//! what was written: [`Diag::with_sinks`] takes any two writers, and
//! [`Diag::from_env`] is the thin wrapper that chooses standard error and a
//! file. A trace file that cannot be opened degrades to one warning on
//! standard error, because a diagnostic that fails a run it was only supposed
//! to describe is a defect in the diagnostic.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Version of the JSON Lines envelope. Version 2 adds identity and ordering.
pub const TRACE_FORMAT_VERSION: u32 = 2;
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
// Keep in-process recorders from repeatedly reacquiring a file's OS lock and
// starving another recorder. The OS lock below also covers other processes.
static TRACE_APPEND_LOCK: Mutex<()> = Mutex::new(());

/// Diagnostic sink failures never alter the application's result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SinkHealth {
    /// Events that could not be written to the human sink.
    pub debug_failures: u64,
    /// Events that could not be written to the JSON sink.
    pub trace_failures: u64,
    /// Last output error, when a sink failed.
    pub last_error: Option<String>,
}

#[derive(Default)]
struct RecorderState {
    sequence: u64,
    health: SinkHealth,
}

/// The variables the recorder reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// Value of `GINARY_DEBUG`; `1` turns the stderr sink on.
    pub ginary_debug: Option<OsString>,
    /// Value of `GINARY_TRACE`; a path turns the JSON Lines sink on.
    pub ginary_trace: Option<OsString>,
}

impl EnvSnapshot {
    /// Reads the relevant variables from the current process environment.
    pub fn from_env() -> Self {
        Self {
            ginary_debug: std::env::var_os("GINARY_DEBUG"),
            ginary_trace: std::env::var_os("GINARY_TRACE"),
        }
    }
}

/// A recorder, holding whichever sinks are switched on.
pub struct Diag {
    /// The human sink, standard error under `GINARY_DEBUG=1`.
    debug: Option<Mutex<Box<dyn Write + Send>>>,
    /// The JSON Lines sink, the `GINARY_TRACE` file.
    trace: Option<Mutex<Box<dyn Write + Send>>>,
    /// When the recorder was built; `t_us` is measured from here.
    ///
    /// [`None`] when nothing is recorded, so a disabled recorder never reads
    /// the clock.
    origin: Option<Instant>,
    run_id: String,
    state: Mutex<RecorderState>,
    sensitive: bool,
}

impl std::fmt::Debug for Diag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diag")
            .field("debug", &self.debug.is_some())
            .field("trace", &self.trace.is_some())
            .finish()
    }
}

impl Diag {
    /// A recorder that records nothing and reads no clock.
    pub fn disabled() -> Self {
        Self {
            debug: None,
            trace: None,
            origin: None,
            run_id: String::new(),
            state: Mutex::new(RecorderState::default()),
            sensitive: false,
        }
    }

    /// A recorder over the sinks a caller supplies.
    ///
    /// Passing [`None`] for both is [`Diag::disabled`].
    pub fn with_sinks(
        debug: Option<Box<dyn Write + Send>>,
        trace: Option<Box<dyn Write + Send>>,
    ) -> Self {
        let enabled = debug.is_some() || trace.is_some();
        if !enabled {
            return Self::disabled();
        }
        Self {
            debug: debug.map(Mutex::new),
            trace: trace.map(Mutex::new),
            origin: enabled.then(Instant::now),
            run_id: format!(
                "{}-{:x}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            state: Mutex::new(RecorderState::default()),
            sensitive: false,
        }
    }

    /// Chooses the sinks the environment asks for.
    ///
    /// Creates the trace file's parent directories. A file that cannot be
    /// opened costs one warning on standard error and leaves the trace sink
    /// off; it is never an error.
    pub fn from_env(env: &EnvSnapshot) -> Self {
        let debug: Option<Box<dyn Write + Send>> = match env.ginary_debug.as_deref() {
            Some(value) if value == "1" => Some(Box::new(std::io::stderr())),
            _ => None,
        };

        let mut open_error = None;
        let trace: Option<Box<dyn Write + Send>> = match env.ginary_trace.as_deref() {
            Some(path) if !path.is_empty() => match open_trace(Path::new(path)) {
                Ok(file) => Some(Box::new(file)),
                Err(error) => {
                    open_error = Some(error.to_string());
                    // One line, and then the run carries on: a diagnostic that
                    // fails a run it was only supposed to describe is a defect
                    // in the diagnostic.
                    let _ = writeln!(
                        std::io::stderr(),
                        "ginary[debug]: GINARY_TRACE={} could not be opened ({}), tracing off",
                        safe_text(&Path::new(path).display().to_string()),
                        safe_text(&error.to_string())
                    );
                    None
                }
            },
            _ => None,
        };

        let recorder = Self::with_sinks(debug, trace)
            .with_sensitive(std::env::var_os("GINARY_TRACE_SENSITIVE").is_some_and(|v| v == "1"));
        if let Some(error) = open_error {
            recorder
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .health = SinkHealth {
                debug_failures: 0,
                trace_failures: 1,
                last_error: Some(error),
            };
        }
        recorder
    }

    /// Explicitly allows argument, environment and credential values in diagnostics.
    /// Keep this off for traces that will be shared with other people.
    pub fn with_sensitive(mut self, enabled: bool) -> Self {
        self.sensitive = enabled;
        self
    }

    /// Returns a snapshot of sink errors, including errors after opening a file.
    pub fn health(&self) -> SinkHealth {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .health
            .clone()
    }

    /// Starts an operation with explicit start and completion records.
    /// Dropping without [`Operation::finish`] records an interrupted operation.
    pub fn operation(&self, name: &str) -> Operation<'_> {
        let id = OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.record_event(name, &[], None, "start", Some(id));
        Operation {
            diag: self,
            name: name.to_owned(),
            start: self.origin.map(|_| Instant::now()),
            finished: false,
            id,
        }
    }

    /// Whether anything is recorded at all.
    pub fn is_enabled(&self) -> bool {
        self.debug.is_some() || self.trace.is_some()
    }

    /// Starts a phase, recorded when the guard is dropped.
    pub fn phase(&self, name: &str) -> PhaseGuard<'_> {
        PhaseGuard {
            diag: self,
            name: name.to_owned(),
            start: self.origin.map(|_| Instant::now()),
        }
    }

    /// Records a fact that is not a phase.
    pub fn kv(&self, phase: &str, kv: &[(&str, &str)]) {
        self.record(phase, kv, None);
    }

    /// Writes one event to whichever sinks are on.
    ///
    /// `t_us` is measured when the record is written rather than when the
    /// event began, so the timestamps are non-decreasing in the order the
    /// lines appear even when phases nest; a phase's start is
    /// `t_us - elapsed_us`.
    ///
    /// A sink that will not take the line is dropped on the floor. The
    /// alternative is a launcher that fails because its trace file filled a
    /// disk.
    fn record(&self, phase: &str, kv: &[(&str, &str)], elapsed_us: Option<u128>) {
        self.record_event(
            phase,
            kv,
            elapsed_us,
            if elapsed_us.is_some() { "end" } else { "fact" },
            None,
        );
    }

    fn record_event(
        &self,
        phase: &str,
        kv: &[(&str, &str)],
        elapsed_us: Option<u128>,
        event: &str,
        operation_id: Option<u64>,
    ) {
        let Some(origin) = self.origin else {
            return;
        };
        // Serialize the clock and both writes together: concurrent callers see
        // the same event order in the human and machine sinks.
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        let sequence = state.sequence;
        let t_us = origin.elapsed().as_micros();
        let values: Vec<(&str, String)> = kv
            .iter()
            .map(|(key, value)| (*key, redact(if self.sensitive { "" } else { key }, value)))
            .collect();

        if let Some(sink) = &self.debug {
            let mut line = format!("ginary[debug]: {}", safe_text(phase));
            for (key, value) in &values {
                line.push_str(&format!(" {}={}", safe_text(key), safe_text(value)));
            }
            if let Some(elapsed) = elapsed_us {
                line.push_str(&format!(" ({elapsed}us)"));
            }
            if let Err(error) = write_line(sink, &line) {
                state.health.debug_failures += 1;
                report_sink_error(&mut state.health, error);
            }
        }

        if let Some(sink) = &self.trace {
            let mut line = format!("{{\"schema_version\":{TRACE_FORMAT_VERSION},\"run_id\":");
            push_json_string(&mut line, &self.run_id);
            if let Some(id) = operation_id {
                line.push_str(&format!(",\"operation_id\":{id}"));
            }
            line.push_str(&format!(
                ",\"sequence\":{sequence},\"event\":\"{event}\",\"t_us\":{t_us},\"phase\":"
            ));
            push_json_string(&mut line, phase);
            line.push_str(",\"kv\":{");
            for (position, (key, value)) in values.iter().enumerate() {
                if position > 0 {
                    line.push(',');
                }
                push_json_string(&mut line, key);
                line.push(':');
                push_json_string(&mut line, value);
            }
            line.push('}');
            if let Some(elapsed) = elapsed_us {
                line.push_str(&format!(",\"elapsed_us\":{elapsed}"));
            }
            line.push('}');
            if let Err(error) = write_line(sink, &line) {
                state.health.trace_failures += 1;
                report_sink_error(&mut state.health, error);
            }
        }
    }
}

/// Opens the `GINARY_TRACE` file for appending, creating its parents.
fn open_trace(path: &Path) -> std::io::Result<TraceFile> {
    let absolute = std::path::absolute(path)?;
    let path = absolute.as_path();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let _append = TRACE_APPEND_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    drop(wait_for_trace(path)?);
    Ok(TraceFile {
        path: path.to_path_buf(),
    })
}

/// One append opens and locks the file for the complete record. Independent
/// recorders and processes therefore cannot interleave fragments of JSON.
struct TraceFile {
    path: PathBuf,
}

impl Write for TraceFile {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _append = TRACE_APPEND_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut file = wait_for_trace(&self.path)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn wait_for_trace(path: &Path) -> std::io::Result<std::fs::File> {
    let started = Instant::now();
    loop {
        match locked_trace(path) {
            Err(error) if started.elapsed() < Duration::from_millis(100) && trace_busy(&error) => {
                std::thread::sleep(Duration::from_millis(1))
            }
            result => return result,
        }
    }
}

fn trace_busy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || cfg!(windows) && error.raw_os_error() == Some(32)
}

fn locked_trace(path: &Path) -> std::io::Result<std::fs::File> {
    if let Ok(metadata) = std::fs::metadata(path)
        && !metadata.is_file()
    {
        return Err(std::io::Error::other(
            "trace destination is not a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    validate_trace_destination(&file)?;
    Ok(file)
}

/// A diagnostic path must not turn an existing artifact or source into a log.
/// Check the first record under the same lock used to append; this also catches
/// replacement of a previously valid destination between events. Reading is
/// bounded independently of the size of a log accumulated over many runs.
fn validate_trace_destination(file: &std::fs::File) -> std::io::Result<()> {
    use std::io::{BufRead, Read};
    const LIMIT: u64 = 1024 * 1024;
    if file.metadata()?.len() == 0 {
        return Ok(());
    }
    let mut first = Vec::new();
    std::io::BufReader::new(file.take(LIMIT + 1)).read_until(b'\n', &mut first)?;
    let valid = first.len() as u64 <= LIMIT
        && first.last() == Some(&b'\n')
        && serde_json::from_slice::<serde_json::Value>(&first)
            .ok()
            .is_some_and(|row| {
                row["phase"].is_string()
                    && row["t_us"].is_u64()
                    && row["kv"].is_object()
                    && (row["schema_version"].is_null()
                        || matches!(row["schema_version"].as_u64(), Some(1 | 2)))
            });
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "refusing to append diagnostics to an existing file without a complete ginary trace record (first record limit: 1 MiB); choose a new trace path",
        ))
    }
}

/// Writes one line to a sink, ignoring a sink that will not take it.
fn write_line(sink: &Mutex<Box<dyn Write + Send>>, line: &str) -> std::io::Result<()> {
    let mut sink = sink.lock().unwrap_or_else(|e| e.into_inner());
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    sink.write_all(&bytes)?;
    sink.flush()
}

fn report_sink_error(health: &mut SinkHealth, error: std::io::Error) {
    if health.last_error.is_none() {
        let _ = writeln!(
            std::io::stderr(),
            "ginary: diagnostic output failed: {}",
            safe_text(&error.to_string())
        );
    }
    health.last_error = Some(error.to_string());
}

/// Escapes terminal controls in untrusted human-readable diagnostic values.
pub fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Redacts common secret-bearing fields and URL credentials in a diagnostic value.
pub fn redact(key: &str, value: &str) -> String {
    let key = key.to_ascii_lowercase();
    if [
        "argv",
        "args",
        "env_set",
        "slogan",
        "cookie",
        "password",
        "secret",
        "token",
        "authorization",
    ]
    .iter()
    .any(|name| key.contains(name))
    {
        return "[redacted]".to_owned();
    }
    let mut result = String::new();
    let mut remaining = value;
    while let Some(scheme) = remaining.find("://") {
        let start = scheme + 3;
        result.push_str(&remaining[..start]);
        let tail = &remaining[start..];
        let mut end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '<' | '>'))
            .unwrap_or(tail.len());
        if let Some(next_scheme) = tail[..end].find("://") {
            let boundary = tail[..next_scheme]
                .rfind(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.'))
                .map_or(0, |offset| offset + 1);
            if boundary > 0 {
                end = boundary;
            }
        }
        let url = &tail[..end];
        let authority_end = url.find(['/', '?', '#']).unwrap_or(url.len());
        result.push_str(url[..authority_end].rsplit('@').next().unwrap_or_default());
        result.push_str(
            url[authority_end..]
                .split(['?', '#'])
                .next()
                .unwrap_or_default(),
        );
        remaining = &tail[end..];
    }
    result.push_str(remaining);
    result
}

/// An explicitly completed operation in a trace.
pub struct Operation<'a> {
    diag: &'a Diag,
    name: String,
    start: Option<Instant>,
    finished: bool,
    id: u64,
}

impl Operation<'_> {
    /// Completes the operation as `end` or `failure`, with structured facts.
    pub fn finish(mut self, success: bool, facts: &[(&str, &str)]) {
        self.diag.record_event(
            &self.name,
            facts,
            self.start.map(|s| s.elapsed().as_micros()),
            if success { "end" } else { "failure" },
            Some(self.id),
        );
        self.finished = true;
    }
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.diag.record_event(
                &self.name,
                &[],
                self.start.map(|s| s.elapsed().as_micros()),
                "interrupted",
                Some(self.id),
            );
        }
    }
}

/// Appends `value` to `out` as a JSON string, quotes and all.
///
/// Hand-written because the recorder writes on the launcher path and a trace
/// line is not a document: it is one string, one map of strings, and two
/// numbers. What it must get right is the escaping, because a value can be a
/// path or the text of an error and neither is under ginary's control. The
/// rules are the ones RFC 8259 requires: the quote, the backslash and every
/// code point below `0x20`.
fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < '\u{20}' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// A phase in progress; dropping it records how long it took.
pub struct PhaseGuard<'a> {
    /// Where the record goes.
    diag: &'a Diag,
    /// The phase name.
    name: String,
    /// When the phase started, or [`None`] when nothing is recorded.
    start: Option<Instant>,
}

impl std::fmt::Debug for PhaseGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhaseGuard")
            .field("name", &self.name)
            .finish()
    }
}

impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            self.diag
                .record(&self.name, &[], Some(start.elapsed().as_micros()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trace value can be a path or the text of an error, so the escaping is
    /// the one part of the hand-written encoder that has to be exhaustive.
    #[test]
    fn a_value_holding_json_punctuation_still_writes_one_object() {
        let sink = std::sync::Arc::new(Mutex::new(Vec::new()));
        struct Shared(std::sync::Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                match self.0.lock() {
                    Ok(mut inner) => {
                        inner.extend_from_slice(buf);
                        Ok(buf.len())
                    }
                    Err(_) => Ok(buf.len()),
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let diag = Diag::with_sinks(None, Some(Box::new(Shared(std::sync::Arc::clone(&sink)))));
        diag.kv(
            "cache",
            &[("path", "/a \"quoted\"\\path\nwith\ta tab\u{1}")],
        );

        let written = sink.lock().expect("not poisoned").clone();
        let text = String::from_utf8(written).expect("the line is UTF-8");
        let object: serde_json::Value =
            serde_json::from_str(text.trim_end()).expect("one JSON object per line");
        assert_eq!(
            object["kv"]["path"],
            "/a \"quoted\"\\path\nwith\ta tab\u{1}"
        );
    }

    #[test]
    fn every_control_character_is_escaped() {
        let mut out = String::new();
        push_json_string(&mut out, "\u{0}\u{8}\u{c}\u{1f}");

        assert_eq!(out, r#""\u0000\b\f\u001f""#);
    }
}
