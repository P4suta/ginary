// SPDX-License-Identifier: MIT OR Apache-2.0
//! Summarising an `erl_crash.dump`.
//!
//! A packaged application that dies writes its crash dump into
//! `<cache>/<app>/`, and the file that lands there is the only account of what
//! the runtime was doing. It is also, routinely, hundreds of megabytes of
//! process, port and heap dumps, and the four facts a person wants are all in
//! the first six lines.
//!
//! `ginary crashdump <path>` reads those, and then the `=proc:` sections, and
//! reports the [`TOP_PROCESSES`] largest by heap. Three rules follow from what
//! the file is.
//!
//! **It is never read into memory.** A dump can be larger than the machine's
//! RAM, so the whole module is a line reader over a [`std::io::BufRead`], with
//! [`MAX_LINE_BYTES`] bounding the value a single line contributes: a dump is
//! a text format written by a process that was already failing, and a line
//! that never ends is one of the ways it fails. The running state is bounded
//! too — the top processes are kept in a list of at most [`TOP_PROCESSES`]
//! entries rather than collected and sorted at the end, so a dump with a
//! million processes costs the same as a dump with ten.
//!
//! **A truncated dump is still an answer.** A runtime killed while writing its
//! dump leaves a file that stops mid-section, which is exactly the case a
//! reader most needs summarised. Everything that was readable is reported and
//! [`CrashDump::truncated`] says the file ended early.
//!
//! **The header is the header.** `Slogan:`, `System version:` and `Taints:`
//! are read only before the first `=section`, because a `=proc:` section has
//! keys of its own — `Name:`, `Spawned as:` — and a reader that matched them
//! anywhere would take the last process's name for the runtime's slogan.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Version of the `crashdump --json` schema.
pub const CRASHDUMP_FORMAT_VERSION: u32 = 1;

/// The tag every `erl_crash.dump` begins with, followed by its version.
pub const CRASH_DUMP_MAGIC: &str = "=erl_crash_dump:";

/// The last line of a dump the runtime finished writing.
pub const END_TAG: &str = "=end";

/// How many processes the summary lists.
pub const TOP_PROCESSES: usize = 5;

/// The longest value the reader will hold out of a single line.
///
/// A `Slogan:` is a term the runtime formatted while it was dying and there is
/// no bound on it in the format. Everything past this is dropped from the
/// value and the line is still consumed, so a dump with one enormous line is
/// summarised rather than allocated.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// The longest key, with its `: `, that the reader will hold in front of a
/// value.
///
/// The buffer one line is read into is [`MAX_LINE_BYTES`] plus this, so that a
/// full [`MAX_LINE_BYTES`] of *value* survives however long the key in front
/// of it was. `Message queue length: ` is the longest key this module knows,
/// and the margin is generous rather than exact: it costs a quarter of a
/// kilobyte and it means a key the format grows later cannot silently start
/// eating into the value.
const MAX_KEY_BYTES: usize = 256;

/// The width the summary's labels are padded to.
///
/// `system version:` is the longest, so every value starts in the same column.
const LABEL_WIDTH: usize = 17;

/// The label of the header line naming the dump format version.
const KEY_SLOGAN: &str = "Slogan:";

/// The label of the header line naming the emulator build.
const KEY_SYSTEM_VERSION: &str = "System version:";

/// The label of the header line listing the loaded NIF and driver taints.
const KEY_TAINTS: &str = "Taints:";

/// The header that opens one process section, followed by its pid.
const SECTION_PROC: &str = "=proc:";

/// The label of the `=proc:` line carrying a registered name.
const KEY_NAME: &str = "Name:";

/// The label of the `=proc:` line carrying the function the process started
/// in.
const KEY_SPAWNED_AS: &str = "Spawned as:";

/// The label of the `=proc:` line carrying the stack and heap size, in words.
const KEY_STACK_HEAP: &str = "Stack+heap:";

/// The longest prefix of an unrecognised first line the refusal quotes.
///
/// A file that is not a crash dump is a file whose bytes nobody vouches for,
/// and the message goes to a terminal. Sixty-four characters is enough to
/// recognise what the file actually is — a shell script, an ELF, a JSON
/// document — and short enough that no file decides how much of somebody's
/// screen it gets.
pub const MAX_FOUND_CHARS: usize = 64;

/// The longest the quoted prefix can be once it has been escaped.
///
/// [`MAX_FOUND_CHARS`] characters, each of which escapes to at most the ten
/// bytes of `\u{10ffff}`, plus the three of the trailing ellipsis. The bound
/// is on the *message*,
/// which is what reaches a reader.
pub const MAX_FOUND_BYTES: usize = MAX_FOUND_CHARS * 10 + ELLIPSIS.len();

/// What is appended when the first line was longer than [`MAX_FOUND_CHARS`].
const ELLIPSIS: &str = "...";

/// What a missing value prints as.
///
/// A reader has to be able to tell an absent value from a missing line, and a
/// line that just stops after its label reads as the second.
const DASH: &str = "-";

/// One process from a `=proc:` section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessSummary {
    /// The pid, as the section header spells it, `<0.42.0>`.
    pub pid: String,
    /// The registered name, when the process has one.
    pub name: Option<String>,
    /// The `Spawned as` line: the function the process started in.
    pub initial_call: Option<String>,
    /// The `Stack+heap` line, in words.
    pub heap: u64,
}

/// What a crash dump says about the run that produced it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrashDump {
    /// Version of this schema; see [`CRASHDUMP_FORMAT_VERSION`].
    pub format_version: u32,
    /// The dump format version, the text after [`CRASH_DUMP_MAGIC`].
    pub dump_version: String,
    /// The second line, which is the date the runtime died.
    pub date: Option<String>,
    /// The `Slogan:` line, which is why it died.
    pub slogan: Option<String>,
    /// The `System version:` line.
    pub system_version: Option<String>,
    /// The `Taints:` line, split on commas, empty when nothing is tainted.
    pub taints: Vec<String>,
    /// How many `=proc:` sections the file holds.
    pub processes: usize,
    /// The [`TOP_PROCESSES`] largest by heap, largest first.
    pub top_processes: Vec<ProcessSummary>,
    /// Whether the file ended in the middle of a section.
    pub truncated: bool,
}

impl CrashDump {
    /// The human-readable summary.
    ///
    /// ```text
    /// dump version:    0.5
    /// date:            Mon Aug 31 11:52:30 2026
    /// slogan:          kaboom
    /// system version:  Erlang/OTP 29 [erts-17.0.5]
    /// taints:          -
    /// processes:       43
    /// truncated:       no
    ///
    /// heap   pid       name       initial call
    /// 6772   <0.44.0>  -          erlang:apply/2
    /// ```
    ///
    /// The table is absent when the dump held no readable process section, so
    /// a heading with nothing under it cannot read as a finding.
    pub fn render_text(&self) -> String {
        let mut text = String::new();
        let mut field = |label: &str, value: &str| {
            text.push_str(&format!("{:<LABEL_WIDTH$}{value}\n", format!("{label}:")));
        };

        field("dump version", or_dash(&self.dump_version));
        field("date", or_dash(self.date.as_deref().unwrap_or_default()));
        field(
            "slogan",
            or_dash(self.slogan.as_deref().unwrap_or_default()),
        );
        field(
            "system version",
            or_dash(self.system_version.as_deref().unwrap_or_default()),
        );
        field("taints", or_dash(&self.taints.join(", ")));
        field("processes", &self.processes.to_string());
        field("truncated", if self.truncated { "yes" } else { "no" });

        if self.top_processes.is_empty() {
            return text;
        }
        let rows: Vec<[String; 4]> = self
            .top_processes
            .iter()
            .map(|process| {
                [
                    process.heap.to_string(),
                    process.pid.clone(),
                    process.name.clone().unwrap_or_else(|| DASH.to_owned()),
                    process
                        .initial_call
                        .clone()
                        .unwrap_or_else(|| DASH.to_owned()),
                ]
            })
            .collect();
        text.push('\n');
        text.push_str(&crate::closure::render_table(
            ["heap", "pid", "name", "initial call"],
            &rows,
        ));
        text
    }
}

/// `value`, or [`DASH`] when it is empty.
fn or_dash(value: &str) -> &str {
    if value.is_empty() { DASH } else { value }
}

/// Reads and summarises the dump at `path`.
///
/// # Errors
///
/// [`CrashdumpError::Io`] when the file cannot be opened or read, and
/// [`CrashdumpError::NotACrashDump`] when it does not begin with
/// [`CRASH_DUMP_MAGIC`].
pub fn read(path: &Path) -> Result<CrashDump, CrashdumpError> {
    let file = std::fs::File::open(path).map_err(|source| CrashdumpError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    match parse(BufReader::new(file)) {
        Err(CrashdumpError::Read { source }) => Err(CrashdumpError::Io {
            path: path.to_path_buf(),
            source,
        }),
        other => other,
    }
}

/// Summarises a dump that is already open.
///
/// The reader is consumed a line at a time and never held, which is what makes
/// a multi-gigabyte dump a bounded read.
///
/// # Errors
///
/// [`CrashdumpError::Read`] when the stream fails, and
/// [`CrashdumpError::NotACrashDump`] when the first line is not the magic.
pub fn parse(src: impl BufRead) -> Result<CrashDump, CrashdumpError> {
    let mut reader = Lines::new(src);

    let first = reader.next_line()?.unwrap_or_default();
    let Some(dump_version) = first.strip_prefix(CRASH_DUMP_MAGIC) else {
        return Err(CrashdumpError::NotACrashDump {
            found: quotable(&first),
        });
    };

    let mut dump = CrashDump {
        format_version: CRASHDUMP_FORMAT_VERSION,
        dump_version: dump_version.trim().to_owned(),
        date: None,
        slogan: None,
        system_version: None,
        taints: Vec::new(),
        processes: 0,
        top_processes: Vec::new(),
        truncated: true,
    };

    let mut header = true;
    let mut current: Option<ProcessSummary> = None;
    // The date has no label: it is simply the line after the magic, which is
    // the one field of the header that cannot be recognised by its key. It is
    // read *inside* the loop rather than in front of it, so that a dump whose
    // second line is already a `=section` — which is what a runtime that died
    // before it wrote a date leaves — opens that section instead of being
    // consumed as a date and dropped. A second line read outside the loop was
    // counted by nothing and left the header scan open, so the first `Slogan:`
    // of the first process became the runtime's.
    let mut date_pending = true;

    while let Some(line) = reader.next_line()? {
        if line.trim().is_empty() {
            date_pending = false;
            continue;
        }
        dump.truncated = line.trim() != END_TAG;

        if let Some(rest) = line.strip_prefix('=') {
            date_pending = false;
            header = false;
            flush(&mut current, &mut dump.top_processes);
            if let Some(pid) = rest
                .strip_prefix(&SECTION_PROC[1..])
                .map(str::trim)
                .filter(|pid| !pid.is_empty())
            {
                dump.processes = dump.processes.saturating_add(1);
                current = Some(ProcessSummary {
                    pid: pid.to_owned(),
                    name: None,
                    initial_call: None,
                    heap: 0,
                });
            }
            continue;
        }

        if std::mem::take(&mut date_pending) {
            dump.date = Some(bounded(&line).to_owned());
            continue;
        }

        if header {
            read_header_line(&line, &mut dump);
        } else if let Some(process) = &mut current {
            read_process_line(&line, process);
        }
    }

    // A section the file stopped inside has still said everything it said, and
    // that is the case this parser most exists for.
    flush(&mut current, &mut dump.top_processes);
    Ok(dump)
}

/// Reads one `Key: value` line of the header.
fn read_header_line(line: &str, dump: &mut CrashDump) {
    if let Some(value) = value_of(line, KEY_SLOGAN) {
        dump.slogan = Some(value.to_owned());
    } else if let Some(value) = value_of(line, KEY_SYSTEM_VERSION) {
        dump.system_version = Some(value.to_owned());
    } else if let Some(value) = value_of(line, KEY_TAINTS) {
        dump.taints = value
            .split(',')
            .map(str::trim)
            .filter(|taint| !taint.is_empty())
            .map(str::to_owned)
            .collect();
    }
}

/// Reads one `Key: value` line of a `=proc:` section.
fn read_process_line(line: &str, process: &mut ProcessSummary) {
    if let Some(value) = value_of(line, KEY_NAME) {
        process.name = Some(value.to_owned());
    } else if let Some(value) = value_of(line, KEY_SPAWNED_AS) {
        process.initial_call = Some(value.to_owned());
    } else if let Some(value) = value_of(line, KEY_STACK_HEAP) {
        // A field that is not a number is a field this reader has nothing to
        // say about; the process is still counted and still listed.
        process.heap = value.parse::<u64>().unwrap_or(0);
    }
}

/// The value of `line` when it begins with `key`, trimmed and bounded.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    Some(bounded(line.strip_prefix(key)?.trim()))
}

/// A first line, cut and escaped so it can be put in a message.
///
/// Two separate jobs, and both are about the reader rather than about the
/// file. The cut is [`MAX_FOUND_CHARS`] characters, so a binary cannot spend
/// a screen; the escaping is [`char::escape_debug`], so a control byte — an
/// ANSI escape sequence above all — reaches the terminal as `\u{1b}` and not
/// as an instruction.
fn quotable(line: &str) -> String {
    let mut found = String::new();
    for (index, character) in line.chars().enumerate() {
        if index >= MAX_FOUND_CHARS {
            found.push_str(ELLIPSIS);
            break;
        }
        found.extend(character.escape_debug());
    }
    found
}

/// `value`, cut to at most [`MAX_LINE_BYTES`].
///
/// The cut lands on a character boundary, so a slogan whose bound falls inside
/// a multi-byte character loses that character rather than becoming something
/// that is not a string.
fn bounded(value: &str) -> &str {
    if value.len() <= MAX_LINE_BYTES {
        return value;
    }
    let mut end = MAX_LINE_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Offers `process` to the running top-`n` list and clears it.
///
/// The list is what keeps the summary bounded: a dump with a million processes
/// never holds more than [`TOP_PROCESSES`] of them. Equal heaps keep the order
/// the file gave them, so the answer is the same on every reader.
fn flush(process: &mut Option<ProcessSummary>, top: &mut Vec<ProcessSummary>) {
    let Some(process) = process.take() else {
        return;
    };
    let position = top
        .iter()
        .position(|held| process.heap > held.heap)
        .unwrap_or(top.len());
    if position >= TOP_PROCESSES {
        return;
    }
    top.insert(position, process);
    top.truncate(TOP_PROCESSES);
}

/// A bounded line reader over a crash dump.
///
/// Every line is consumed to its end and at most [`MAX_LINE_BYTES`] plus
/// [`MAX_KEY_BYTES`] of it is kept, so the reader's memory does not depend on
/// what the dying runtime happened to write. Bytes that are not UTF-8 are
/// replaced rather than refused: a slogan is a formatted Erlang term and may
/// hold anything a binary held.
struct Lines<R> {
    /// The stream the dump is read from.
    src: R,
    /// The buffer one line is read into, reused across lines.
    buffer: Vec<u8>,
}

impl<R: BufRead> Lines<R> {
    /// A reader over `src`.
    fn new(src: R) -> Self {
        Self {
            src,
            buffer: Vec::new(),
        }
    }

    /// The next line without its terminator, or `None` at the end.
    fn next_line(&mut self) -> Result<Option<String>, CrashdumpError> {
        self.buffer.clear();
        let limit = MAX_LINE_BYTES.saturating_add(MAX_KEY_BYTES);
        let mut any = false;
        loop {
            let available = match self.src.fill_buf() {
                Ok(available) => available,
                Err(source) if source.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(source) => return Err(CrashdumpError::Read { source }),
            };
            if available.is_empty() {
                break;
            }
            any = true;
            let (chunk, consumed, done) = match available.iter().position(|byte| *byte == b'\n') {
                Some(newline) => (&available[..newline], newline + 1, true),
                None => (available, available.len(), false),
            };
            let room = limit.saturating_sub(self.buffer.len());
            self.buffer
                .extend_from_slice(&chunk[..room.min(chunk.len())]);
            self.src.consume(consumed);
            if done {
                break;
            }
        }
        if !any && self.buffer.is_empty() {
            return Ok(None);
        }

        let mut line = String::from_utf8_lossy(&self.buffer).into_owned();
        if line.ends_with('\r') {
            line.pop();
        }
        Ok(Some(line))
    }
}

/// Why a crash dump could not be summarised.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CrashdumpError {
    /// The file could not be opened.
    #[error("cannot read {path}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The stream failed part-way through.
    #[error("the crash dump could not be read to its end")]
    Read {
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The first line is not the magic, so the file is something else.
    #[error("the file begins `{found}`, and an erl_crash.dump begins `=erl_crash_dump:`")]
    NotACrashDump {
        /// The first line, cut to [`MAX_FOUND_CHARS`] characters and escaped;
        /// see [`MAX_FOUND_BYTES`] for the bound on the message.
        found: String,
    },
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// The smallest whole dump: a header and nothing else.
    fn header_only(body: &str) -> CrashDump {
        parse(Cursor::new(
            format!("=erl_crash_dump:0.5\n{body}").into_bytes(),
        ))
        .expect("a dump with a header is a dump")
    }

    #[test]
    fn an_empty_file_is_not_a_crash_dump() {
        let error = parse(Cursor::new(Vec::new())).expect_err("nothing is not a dump");
        assert!(matches!(error, CrashdumpError::NotACrashDump { found } if found.is_empty()));
    }

    #[test]
    fn a_dump_with_no_second_line_is_truncated() {
        let dump = header_only("");
        assert!(dump.truncated);
        assert_eq!(dump.date, None);
        assert_eq!(dump.processes, 0);
        assert_eq!(
            dump.render_text().lines().count(),
            7,
            "no table, no heading"
        );
    }

    #[test]
    fn a_slogan_that_is_not_the_last_line_still_leaves_the_dump_truncated() {
        let dump = header_only("Mon\nSlogan: boom\n");
        assert_eq!(dump.slogan.as_deref(), Some("boom"));
        assert!(dump.truncated, "nothing said `=end`");
    }

    #[test]
    fn a_process_key_is_never_read_as_a_header_key() {
        let dump = header_only("Mon\n=proc:<0.1.0>\nSlogan: not the runtime's\n=end\n");
        assert_eq!(dump.slogan, None);
        assert_eq!(dump.processes, 1);
        assert!(!dump.truncated);
    }

    #[test]
    fn a_heap_that_is_not_a_number_still_lists_the_process() {
        let dump = header_only("Mon\n=proc:<0.1.0>\nStack+heap: unknown\n=end\n");
        assert_eq!(dump.top_processes.len(), 1);
        assert_eq!(dump.top_processes[0].heap, 0);
    }

    #[test]
    fn only_the_largest_processes_are_ever_held() {
        let mut text = String::from("=erl_crash_dump:0.5\nMon\n");
        for index in 0..50u64 {
            text.push_str(&format!(
                "=proc:<0.{index}.0>\nStack+heap: {}\n",
                index * 10
            ));
        }
        text.push_str("=end\n");

        let dump = parse(Cursor::new(text.into_bytes())).expect("a dump");

        assert_eq!(dump.processes, 50);
        assert_eq!(dump.top_processes.len(), TOP_PROCESSES);
        assert_eq!(dump.top_processes[0].heap, 490);
        assert_eq!(dump.top_processes[TOP_PROCESSES - 1].heap, 450);
    }

    #[test]
    fn processes_of_one_size_keep_the_order_the_file_gave_them() {
        let mut text = String::from("=erl_crash_dump:0.5\nMon\n");
        for index in 0..3u64 {
            text.push_str(&format!("=proc:<0.{index}.0>\nStack+heap: 7\n"));
        }
        text.push_str("=end\n");

        let dump = parse(Cursor::new(text.into_bytes())).expect("a dump");

        assert_eq!(
            dump.top_processes
                .iter()
                .map(|process| process.pid.as_str())
                .collect::<Vec<_>>(),
            ["<0.0.0>", "<0.1.0>", "<0.2.0>"]
        );
    }

    #[test]
    fn a_line_with_no_terminator_at_the_end_of_the_file_is_still_a_line() {
        let dump = header_only("Mon\nSlogan: boom");
        assert_eq!(dump.slogan.as_deref(), Some("boom"));
    }

    #[test]
    fn carriage_returns_are_not_part_of_a_value() {
        let dump = header_only("Mon\r\nSlogan: boom\r\n=end\r\n");
        assert_eq!(dump.slogan.as_deref(), Some("boom"));
        assert!(!dump.truncated);
    }

    #[test]
    fn an_unrecognised_first_line_is_cut_and_escaped_before_it_is_a_message() {
        let error = parse(Cursor::new("\u{1b}[31m".repeat(1_000).into_bytes()))
            .expect_err("a terminal escape sequence is not a dump");

        let CrashdumpError::NotACrashDump { found } = error else {
            panic!("expected NotACrashDump, got {error:?}")
        };
        assert!(
            found.len() <= MAX_FOUND_BYTES,
            "{} bytes of the file reached the message",
            found.len()
        );
        assert!(found.ends_with(ELLIPSIS), "{found}");
        assert!(
            !found.chars().any(char::is_control),
            "a control character survived: {found:?}"
        );
    }

    #[test]
    fn an_empty_taints_line_is_no_taints() {
        let dump = header_only("Mon\nTaints: \n=end\n");
        assert_eq!(dump.taints, Vec::<String>::new());
        assert!(dump.render_text().contains("taints:          -"));
    }
}
