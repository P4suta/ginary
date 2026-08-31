// SPDX-License-Identifier: MIT OR Apache-2.0
//! The second line of a crash dump was read as the date and nothing else.
//!
//! `crashdump::parse` handled the line after `=erl_crash_dump:` outside its
//! loop: it became the date when it was neither empty nor a `=section`, and
//! was otherwise dropped. A dump whose *second* line is already a section —
//! which is what a runtime that died before writing a date line leaves — was
//! therefore swallowed whole. The section was never counted, and, because the
//! loop was entered with the header scan still open, the first `Slogan:` inside
//! that process section became the runtime's slogan: exactly the confusion the
//! module's own documentation says cannot happen.
//!
//! The correct behaviour is that every line after the magic goes through one
//! path. A section on the second line opens that section, closes the header,
//! and is counted; the date stays `None`, because there was none.

use std::io::Cursor;

use ginary::crashdump;

/// A dump with no date line: the first thing after the magic is a process.
const DUMP: &str = "=erl_crash_dump:0.5\n\
                    =proc:<0.1.0>\n\
                    Name: worker\n\
                    Slogan: not the runtime\n\
                    Stack+heap: 42\n\
                    =proc:<0.2.0>\n\
                    Stack+heap: 7\n\
                    =end\n";

#[test]
fn a_section_on_the_second_line_opens_that_section() {
    let dump = crashdump::parse(Cursor::new(DUMP.as_bytes().to_vec()))
        .expect("a dump with a magic line is a dump");

    assert_eq!(dump.date, None, "there is no date line to read");
    assert_eq!(
        dump.processes, 2,
        "the section on the second line is a section"
    );
    assert_eq!(
        dump.slogan, None,
        "`Slogan:` inside a `=proc:` section is the process's, not the runtime's"
    );
    assert_eq!(
        dump.top_processes
            .iter()
            .map(|process| (process.pid.as_str(), process.heap))
            .collect::<Vec<_>>(),
        [("<0.1.0>", 42), ("<0.2.0>", 7)]
    );
    assert_eq!(dump.top_processes[0].name.as_deref(), Some("worker"));
    assert!(!dump.truncated, "the file ends with `=end`");
}
