// SPDX-License-Identifier: MIT OR Apache-2.0
//! A dependency-free event recorder for the launcher and, later, for
//! `ginary build -v`.
//!
//! The launcher has to be explainable without being slow. Two environment
//! variables turn it on and nothing else does:
//!
//! - `GINARY_DEBUG=1` writes human lines to standard error,
//!   `ginary[debug]: <phase> <k=v ...> (<elapsed_us>us)`;
//! - `GINARY_TRACE=<path>` appends JSON Lines to that file,
//!   `{"t_us":..,"phase":..,"kv":{..}}`, one object per line, in write order.
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
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

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
        Self {
            debug: debug.map(Mutex::new),
            trace: trace.map(Mutex::new),
            origin: enabled.then(Instant::now),
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

        let trace: Option<Box<dyn Write + Send>> = match env.ginary_trace.as_deref() {
            Some(path) if !path.is_empty() => match open_trace(Path::new(path)) {
                Ok(file) => Some(Box::new(file)),
                Err(error) => {
                    // One line, and then the run carries on: a diagnostic that
                    // fails a run it was only supposed to describe is a defect
                    // in the diagnostic.
                    let _ = writeln!(
                        std::io::stderr(),
                        "ginary[debug]: GINARY_TRACE={} could not be opened ({error}), tracing off",
                        Path::new(path).display()
                    );
                    None
                }
            },
            _ => None,
        };

        Self::with_sinks(debug, trace)
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
        let Some(origin) = self.origin else {
            return;
        };
        let t_us = origin.elapsed().as_micros();

        if let Some(sink) = &self.debug {
            let mut line = format!("ginary[debug]: {phase}");
            for (key, value) in kv {
                line.push_str(&format!(" {key}={value}"));
            }
            if let Some(elapsed) = elapsed_us {
                line.push_str(&format!(" ({elapsed}us)"));
            }
            write_line(sink, &line);
        }

        if let Some(sink) = &self.trace {
            let mut line = format!("{{\"t_us\":{t_us},\"phase\":");
            push_json_string(&mut line, phase);
            line.push_str(",\"kv\":{");
            for (position, (key, value)) in kv.iter().enumerate() {
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
            write_line(sink, &line);
        }
    }
}

/// Opens the `GINARY_TRACE` file for appending, creating its parents.
fn open_trace(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// Writes one line to a sink, ignoring a sink that will not take it.
fn write_line(sink: &Mutex<Box<dyn Write + Send>>, line: &str) {
    if let Ok(mut sink) = sink.lock() {
        let _ = writeln!(sink, "{line}");
        let _ = sink.flush();
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
