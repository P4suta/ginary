// SPDX-License-Identifier: MIT OR Apache-2.0
//! A parser for the subset of Erlang terms that appears in an `.app` file.
//!
//! An OTP application resource file holds exactly one term,
//! `{application, Name, Props}`, written in Erlang's term syntax. ginary has to
//! read those files to compute the application closure, and it has to do so
//! without an Erlang installation: at build time `erl` may be absent, and on
//! the launcher path running a program is out of the question. So this module
//! implements the grammar directly rather than shelling out to
//! `file:consult/1`.
//!
//! The supported grammar is deliberately small — [`parse_terms`] documents it
//! in full. Anything outside it (maps, funs, variables, list tails) is a
//! [`ParseError`] that names the construct, never a panic.
//!
//! The design is a single hand-written recursive-descent pass over `Vec<char>`
//! rather than a separate lexer over bytes. Two reasons: an error has to carry
//! a line and a *character* column, which a byte offset cannot give on a file
//! with a non-ASCII comment; and the grammar is small enough that a token
//! stream would be more code than the parser it feeds.
//!
//! Reading a file is two steps, and they fail differently on purpose.
//! [`parse_terms`] answers "is this Erlang?"; `TryFrom<&[Term]>` answers "is
//! this an application resource?". Only the second knows about `vsn`, and only
//! the first knows about line numbers.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// One Erlang term from the supported subset.
///
/// Character literals (`$a`) have no variant of their own: Erlang defines them
/// as integers, so `$a` parses to `Term::Int(97)` and re-serialises as `97`.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    /// An atom, holding the *unquoted* name: `'my-app'` is `Atom("my-app")`.
    Atom(String),
    /// A string, holding the *unescaped* contents.
    Str(String),
    /// A binary written as `<<"...">>`, holding the unescaped contents.
    ///
    /// Every character of a parsed binary is a byte, that is, at most 255;
    /// nothing stops a caller building one that is not, and `Display` then
    /// writes a literal Erlang would reject.
    Bin(String),
    /// An integer, including character literals.
    Int(i64),
    /// A float.
    Float(f64),
    /// A tuple, `{}` included.
    Tuple(Vec<Term>),
    /// A proper list, `[]` included. Improper lists are not supported.
    List(Vec<Term>),
}

impl fmt::Display for Term {
    /// Re-serialises the term as Erlang source that [`parse_terms`] accepts.
    ///
    /// This is the inverse of parsing for every term this module can produce,
    /// which is what the round-trip property test asserts. It is meant for
    /// diagnostics: the output is a single line with `, ` between elements.
    ///
    /// Two terms have no Erlang literal, and this module's parser produces
    /// neither: a non-finite float, written the way Rust writes `inf` and
    /// `NaN`, and a [`Term::Bin`] holding a character above 255, written as the
    /// binary literal Erlang rejects.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Atom(name) => {
                if is_bare_atom(name) {
                    f.write_str(name)
                } else {
                    write!(f, "'{}'", escape(name, '\''))
                }
            }
            Self::Str(text) => write!(f, "\"{}\"", escape(text, '"')),
            Self::Bin(text) if text.is_empty() => f.write_str("<<>>"),
            Self::Bin(text) => write!(f, "<<\"{}\">>", escape(text, '"')),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => f.write_str(&render_float(*value)),
            Self::Tuple(items) => write!(f, "{{{}}}", render_items(items)),
            Self::List(items) => write!(f, "[{}]", render_items(items)),
        }
    }
}

/// Renders the elements of a tuple or list, separated by `, `.
fn render_items(items: &[Term]) -> String {
    items
        .iter()
        .map(Term::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The words the Erlang grammar reserves, which are never bare atoms.
///
/// `fun` is the one this module's own parser rejects, but none of the others is
/// an atom to a compiler either, so all of them are quoted on the way out.
/// Sorted, because [`is_reserved_word`] binary-searches it.
const RESERVED_WORDS: [&str; 29] = [
    "after", "and", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor", "case",
    "catch", "cond", "div", "else", "end", "fun", "if", "let", "maybe", "not", "of", "or",
    "orelse", "receive", "rem", "try", "when", "xor",
];

/// Whether `name` is one of [`RESERVED_WORDS`].
fn is_reserved_word(name: &str) -> bool {
    RESERVED_WORDS.binary_search(&name).is_ok()
}

/// Whether an atom can be written without quotes.
///
/// The rule is narrower than the one the parser accepts: quoting is always
/// valid, so `Display` only leaves the quotes off where every reader agrees.
/// A reserved word is not an atom at all — `fun` opens a construct, `end`
/// closes one — so it is quoted however ordinary its spelling looks.
fn is_bare_atom(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@')
        && !is_reserved_word(name)
}

/// Escapes the contents of a quoted atom, a string or a binary.
///
/// `quote` is the delimiter that has to be escaped; the backslash and the ASCII
/// control characters always are. A control character is written as a
/// three-digit octal escape, which the parser reads back exactly because the
/// length is fixed.
fn escape(text: &str, quote: char) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii_control() => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Renders a float the way Erlang writes one, so that it can be read back.
///
/// Rust's shortest round-trip form is not always valid Erlang: it prints `1e-7`
/// where Erlang needs a digit on both sides of the dot, `1.0e-7`. The digits
/// themselves are Rust's, so the value survives the round trip exactly.
fn render_float(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    let text = format!("{value:?}");
    match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) if !mantissa.contains('.') => {
            format!("{mantissa}.0e{exponent}")
        }
        Some(_) => text,
        None if text.contains('.') => text,
        None => format!("{text}.0"),
    }
}

/// A position in the source, with what the parser wanted and what it saw.
///
/// `line` and `col` are 1-based and count characters, not bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The 1-based line the offending token starts on.
    pub line: u32,
    /// The 1-based column the offending token starts at.
    pub col: u32,
    /// What the parser would have accepted, as backticked tokens or a phrase.
    pub expected: String,
    /// What it found instead, as a backticked token or a named construct.
    pub found: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: expected {}, found {}",
            self.line, self.col, self.expected, self.found
        )
    }
}

impl std::error::Error for ParseError {}

/// Parses a sequence of top-level terms, each terminated by `.`.
///
/// The grammar, in full:
///
/// - `%` starts a comment that runs to the end of the line;
/// - bare atoms start with a lowercase letter and continue with letters,
///   digits, `_` and `@`; "lowercase" is Unicode's, as Erlang's is Latin-1's,
///   so `é` is an atom and not a syntax error;
/// - quoted atoms are `'...'` with `\'`, `\\`, `\n`, `\t`, `\xNN`, `\x{...}`
///   and octal escapes;
/// - strings are `"..."` with the same escapes, without adjacent-string
///   concatenation;
/// - binaries are `<<"...">>` and `<<>>`, and hold bytes: a character above
///   255 needs a `/utf8` segment, which this subset does not have, so it is an
///   error here exactly as it is to `erlc`;
/// - integers are decimal with an optional leading `-`, without the `16#FF`
///   base syntax;
/// - floats are `1.5` and `-2.0e3`, always with a digit on each side of the
///   dot;
/// - character literals are `$c` and `$\n`, and yield integers;
/// - tuples are `{}` and lists are `[]`, without the `[H|T]` tail syntax;
/// - each top-level term ends with `.` followed by whitespace or end of input.
///
/// Anything else — maps, funs, variables, a `|` tail — is an error naming the
/// construct. So is nesting more than a hundred tuples and lists deep: the
/// parser recurses, and an `.app` file is not a reason to run out of stack.
///
/// # Errors
///
/// [`ParseError`], carrying the line, the column, what was expected and what
/// was found, on the first construct that does not fit the grammar above.
pub fn parse_terms(src: &str) -> Result<Vec<Term>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut parser = Parser::new(&chars);
    let mut terms = Vec::new();

    loop {
        parser.skip_trivia();
        if parser.peek().is_none() {
            break;
        }
        let term = parser.parse_term()?;
        parser.skip_trivia();
        parser.expect_full_stop()?;
        terms.push(term);
    }

    Ok(terms)
}

/// How deeply tuples and lists may nest before the parser gives up.
///
/// The parser descends recursively, so without a bound a file of nothing but
/// open brackets would exhaust the stack, and a stack overflow is an abort with
/// no message rather than an error a caller can report. Real resource files
/// nest fewer than ten deep; a hundred is far past anything OTP or Gleam emits.
const MAX_NESTING: usize = 100;

/// The parser state: the source as characters, and where we are in it.
struct Parser<'a> {
    /// The whole source. Indexing by character is what makes the column count
    /// characters rather than bytes.
    chars: &'a [char],
    /// The index of the next character to read.
    index: usize,
    /// The 1-based line of `index`.
    line: u32,
    /// The 1-based column of `index`.
    col: u32,
    /// How many tuples and lists are open around `index`.
    depth: usize,
}

impl<'a> Parser<'a> {
    /// A parser positioned at the first character.
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            index: 0,
            line: 1,
            col: 1,
            depth: 0,
        }
    }

    /// The next character, without consuming it.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    /// The character `ahead` positions past the next one.
    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.index + ahead).copied()
    }

    /// Consumes one character, keeping the line and column in step.
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.index += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// The current position.
    fn at(&self) -> (u32, u32) {
        (self.line, self.col)
    }

    /// An error at the current position, describing what is there.
    fn error(&self, expected: &str) -> ParseError {
        self.error_at(self.at(), expected)
    }

    /// An error at a remembered position, describing what is *now* there.
    fn error_at(&self, (line, col): (u32, u32), expected: &str) -> ParseError {
        ParseError {
            line,
            col,
            expected: expected.to_owned(),
            found: self.describe_found(),
        }
    }

    /// Names the construct at the current position, for an error message.
    ///
    /// The unsupported constructs are named rather than quoted, because
    /// ``found `#` `` would send the reader looking for a typo instead of
    /// telling them that maps are outside the subset.
    fn describe_found(&self) -> String {
        match self.peek() {
            None => "end of input".to_owned(),
            Some('#') if self.peek_at(1) == Some('{') => "a map (`#{`)".to_owned(),
            Some('|') => "a list tail (`|`)".to_owned(),
            Some(c) if is_variable_start(c) => format!("a variable (`{}`)", self.word()),
            Some(c) if is_atom_start(c) => {
                let word = self.word();
                if word == "fun" {
                    "a fun (`fun`)".to_owned()
                } else {
                    format!("`{word}`")
                }
            }
            Some(c) => format!("`{c}`"),
        }
    }

    /// Whether the identifier at the current position is exactly `expected`.
    ///
    /// Reserved words are recognised on the hot path — every atom passes
    /// through here — so this compares in place rather than building a string.
    fn word_is(&self, expected: &str) -> bool {
        let mut source = self.chars[self.index..].iter().copied();
        for want in expected.chars() {
            if source.next() != Some(want) {
                return false;
            }
        }
        !source.next().is_some_and(is_name_char)
    }

    /// The identifier at the current position, without consuming it.
    fn word(&self) -> String {
        self.chars[self.index..]
            .iter()
            .take_while(|c| is_name_char(**c))
            .collect()
    }

    /// Consumes the identifier at the current position.
    fn take_word(&mut self) -> String {
        let mut word = String::new();
        while self.peek().is_some_and(is_name_char) {
            if let Some(c) = self.bump() {
                word.push(c);
            }
        }
        word
    }

    /// Skips whitespace and `%` comments, together, until neither is next.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('%') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    /// Consumes the `.` that ends a top-level term.
    ///
    /// Erlang ends a term with a full stop *followed by whitespace*, which is
    /// what keeps `1.5` from being the integer `1` and an empty term.
    fn expect_full_stop(&mut self) -> Result<(), ParseError> {
        if self.peek() != Some('.') {
            return Err(self.error("`.`"));
        }
        match self.peek_at(1) {
            None => {}
            Some(c) if c.is_whitespace() || c == '%' => {}
            Some(_) => return Err(self.error("`.` followed by whitespace or end of input")),
        }
        self.bump();
        Ok(())
    }

    /// Parses one term, skipping any leading whitespace and comments.
    fn parse_term(&mut self) -> Result<Term, ParseError> {
        self.skip_trivia();
        match self.peek() {
            Some('{') => self.parse_sequence('}', Term::Tuple),
            Some('[') => self.parse_sequence(']', Term::List),
            Some('"') => self.parse_delimited('"', "string").map(Term::Str),
            Some('\'') => self.parse_delimited('\'', "atom").map(Term::Atom),
            Some('<') if self.peek_at(1) == Some('<') => self.parse_binary(),
            Some('$') => self.parse_char(),
            Some('-') if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => self.parse_number(),
            Some(c) if c.is_ascii_digit() => self.parse_number(),
            // `fun` is a reserved word, not an atom, and it opens a construct
            // this subset does not have.
            Some(c) if is_atom_start(c) && !self.word_is("fun") => Ok(Term::Atom(self.take_word())),
            _ => Err(self.error("a term")),
        }
    }

    /// Parses a `{...}` or `[...]`, given its closing delimiter.
    fn parse_sequence(
        &mut self,
        close: char,
        build: fn(Vec<Term>) -> Term,
    ) -> Result<Term, ParseError> {
        if self.depth >= MAX_NESTING {
            return Err(self.error(&format!("at most {MAX_NESTING} levels of nesting")));
        }
        self.depth += 1;
        self.bump();
        self.skip_trivia();
        if self.peek() == Some(close) {
            self.bump();
            self.depth -= 1;
            return Ok(build(Vec::new()));
        }

        let mut items = Vec::new();
        loop {
            items.push(self.parse_term()?);
            self.skip_trivia();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(c) if c == close => {
                    self.bump();
                    break;
                }
                _ => return Err(self.error(&format!("`,` or `{close}`"))),
            }
        }
        self.depth -= 1;
        Ok(build(items))
    }

    /// Parses a `'...'` or `"..."`, returning the unescaped contents.
    ///
    /// `kind` names the construct in the message an unterminated literal
    /// produces, and that message is reported at the *opening* quote: the end
    /// of the file is where the parser noticed, not where the mistake is.
    fn parse_delimited(&mut self, quote: char, kind: &str) -> Result<String, ParseError> {
        let start = self.at();
        self.bump();
        let mut text = String::new();
        loop {
            match self.bump() {
                None => {
                    return Err(ParseError {
                        line: start.0,
                        col: start.1,
                        expected: format!("a closing `{quote}`"),
                        found: format!("an unterminated {kind}"),
                    });
                }
                Some(c) if c == quote => break,
                Some('\\') => text.push(self.parse_escape()?),
                Some(c) => text.push(c),
            }
        }
        Ok(text)
    }

    /// Parses one escape sequence, the backslash already consumed.
    fn parse_escape(&mut self) -> Result<char, ParseError> {
        let start = self.at();
        let Some(c) = self.bump() else {
            return Err(self.error_at(start, "an escape sequence"));
        };
        let escaped = match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'v' => '\u{b}',
            'e' => '\u{1b}',
            's' => ' ',
            'd' => '\u{7f}',
            '0'..='7' => self.parse_octal_escape(c, start)?,
            'x' => self.parse_hex_escape(start)?,
            // `\\`, `\'`, `\"` and, as in Erlang, any other character stands
            // for itself.
            other => other,
        };
        Ok(escaped)
    }

    /// Parses the rest of a `\NNN` escape, `first` already consumed.
    fn parse_octal_escape(&mut self, first: char, start: (u32, u32)) -> Result<char, ParseError> {
        let mut value = digit_value(first, 8).unwrap_or(0);
        for _ in 0..2 {
            let Some(digit) = self.peek().and_then(|c| digit_value(c, 8)) else {
                break;
            };
            value = value * 8 + digit;
            self.bump();
        }
        char::from_u32(value).ok_or_else(|| ParseError {
            line: start.0,
            col: start.1,
            expected: "an escape naming a character".to_owned(),
            found: format!("`\\{value:o}`"),
        })
    }

    /// Parses a `\xNN` or `\x{...}` escape, the `x` already consumed.
    fn parse_hex_escape(&mut self, start: (u32, u32)) -> Result<char, ParseError> {
        let mut value = 0_u32;
        if self.peek() == Some('{') {
            self.bump();
            let mut digits = 0_u32;
            while let Some(digit) = self.peek().and_then(|c| digit_value(c, 16)) {
                value = value.saturating_mul(16).saturating_add(digit);
                digits += 1;
                self.bump();
            }
            if digits == 0 || self.peek() != Some('}') {
                return Err(self.error_at(start, "a `\\x{...}` escape"));
            }
            self.bump();
        } else {
            for _ in 0..2 {
                let Some(digit) = self.peek().and_then(|c| digit_value(c, 16)) else {
                    return Err(self.error_at(start, "two hexadecimal digits after `\\x`"));
                };
                value = value * 16 + digit;
                self.bump();
            }
        }
        char::from_u32(value).ok_or_else(|| ParseError {
            line: start.0,
            col: start.1,
            expected: "an escape naming a character".to_owned(),
            found: format!("`\\x{value:x}`"),
        })
    }

    /// Parses `$c` or `$\n`, which Erlang defines as integers.
    fn parse_char(&mut self) -> Result<Term, ParseError> {
        let start = self.at();
        self.bump();
        match self.bump() {
            None => Err(self.error_at(start, "a character after `$`")),
            Some('\\') => Ok(Term::Int(i64::from(u32::from(self.parse_escape()?)))),
            Some(c) => Ok(Term::Int(i64::from(u32::from(c)))),
        }
    }

    /// Parses `<<"...">>` or `<<>>`.
    ///
    /// The contents are bytes. `<<"é">>` is the single byte 233 and is fine;
    /// `<<"\u{1f600}">>` is not a binary at all without a `/utf8` segment, and
    /// is reported at the opening quote rather than parsed into a term that
    /// could never be written back.
    fn parse_binary(&mut self) -> Result<Term, ParseError> {
        self.bump();
        self.bump();
        self.skip_trivia();
        if self.peek() == Some('>') && self.peek_at(1) == Some('>') {
            self.bump();
            self.bump();
            return Ok(Term::Bin(String::new()));
        }
        if self.peek() != Some('"') {
            return Err(self.error("a string or `>>`"));
        }
        let start = self.at();
        let text = self.parse_delimited('"', "string")?;
        if let Some(c) = text.chars().find(|c| (*c as u32) > u32::from(u8::MAX)) {
            let (line, col) = start;
            return Err(ParseError {
                line,
                col,
                expected: "a binary of bytes, each character up to 255".to_owned(),
                found: format!("`{c}`, which is codepoint {}", c as u32),
            });
        }
        self.skip_trivia();
        if self.peek() != Some('>') || self.peek_at(1) != Some('>') {
            return Err(self.error("`>>`"));
        }
        self.bump();
        self.bump();
        Ok(Term::Bin(text))
    }

    /// Parses an integer or a float, with an optional leading `-`.
    ///
    /// A `.` only belongs to the number when a digit follows it, which is what
    /// keeps the `.` that ends a top-level term out of `42.`.
    fn parse_number(&mut self) -> Result<Term, ParseError> {
        let start = self.at();
        let begin = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        self.take_digits();

        let mut is_float = false;
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.bump();
            self.take_digits();
            self.take_exponent();
        }

        let text: String = self.chars[begin..self.index].iter().collect();
        if is_float {
            return match text.parse::<f64>() {
                Ok(value) if value.is_finite() => Ok(Term::Float(value)),
                _ => Err(ParseError {
                    line: start.0,
                    col: start.1,
                    expected: "a float a 64-bit float can hold".to_owned(),
                    found: format!("`{text}`"),
                }),
            };
        }
        text.parse::<i64>().map(Term::Int).map_err(|_| ParseError {
            line: start.0,
            col: start.1,
            expected: "an integer a 64-bit integer can hold".to_owned(),
            found: format!("`{text}`"),
        })
    }

    /// Consumes a run of decimal digits.
    fn take_digits(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
    }

    /// Consumes `e[+-]?<digits>`, but only if it is really an exponent.
    ///
    /// `1.0e` and `1.0end` are not: leaving the `e` alone lets the caller
    /// report the real problem instead of a broken number.
    fn take_exponent(&mut self) {
        if !matches!(self.peek(), Some('e' | 'E')) {
            return;
        }
        let signed = matches!(self.peek_at(1), Some('+' | '-'));
        let digit = usize::from(signed) + 1;
        if !self.peek_at(digit).is_some_and(|c| c.is_ascii_digit()) {
            return;
        }
        self.bump();
        if signed {
            self.bump();
        }
        self.take_digits();
    }
}

/// The value of `digit` in the given base, if it is one.
fn digit_value(digit: char, base: u32) -> Option<u32> {
    digit.to_digit(base)
}

/// Whether a character can start a bare atom.
///
/// Erlang's rule is "a lowercase Latin-1 letter"; Unicode's `is_lowercase` is
/// the closest thing in the standard library and is a superset, so a file that
/// `erl` accepts is accepted here.
fn is_atom_start(c: char) -> bool {
    c.is_lowercase()
}

/// Whether a character can start a variable, which this subset rejects.
fn is_variable_start(c: char) -> bool {
    c.is_uppercase() || c == '_'
}

/// Whether a character can continue an atom or a variable.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '@'
}

/// What ginary needs to know about one OTP application resource file.
///
/// Keys ginary does not use (`runtime_dependencies`, `optional_applications`,
/// `maxT`, …) are parsed and discarded; only `env` is summarised, by key, since
/// the values can be arbitrarily deep and nothing downstream reads them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppResource {
    /// The application name, from `{application, Name, _}`.
    pub name: String,
    /// The `vsn` property. Required.
    pub vsn: String,
    /// The `description` property, if the file has one.
    pub description: Option<String>,
    /// The `applications` property, in file order.
    pub applications: Vec<String>,
    /// The `optional_applications` property, in file order.
    ///
    /// Every name here also appears in `applications`, by OTP's own rule, but
    /// it may legitimately be absent at run time. The closure has to know the
    /// difference, so the key is read rather than dropped.
    pub optional_applications: Vec<String>,
    /// The `included_applications` property, in file order.
    pub included_applications: Vec<String>,
    /// The `modules` property, in file order.
    pub modules: Vec<String>,
    /// The `registered` property, in file order.
    pub registered: Vec<String>,
    /// Whether the file has a `mod` property, that is, whether starting the
    /// application runs an application callback module.
    pub has_mod: bool,
    /// The keys of the `env` property, in file order.
    pub env_keys: Vec<String>,
    /// Recoverable problems found while reading the file, in the order they
    /// were found. A duplicate key is the canonical case: the last value wins
    /// and the shadowing is recorded here rather than being silently accepted.
    pub warnings: Vec<String>,
}

/// Why an `.app` file could not be turned into an [`AppResource`].
#[derive(Debug, thiserror::Error)]
pub enum AppFileError {
    /// The file could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid Erlang term syntax.
    #[error("{path}: {source}")]
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        /// Where and why it failed.
        #[source]
        source: ParseError,
    },
    /// The file does not hold a `{application, Name, Props}` term.
    #[error("expected one `{{application, Name, Props}}` term, found {found}")]
    NotAnApplication {
        /// A description of what was found instead.
        found: String,
    },
    /// The file holds more than one term.
    #[error("expected exactly one `{{application, Name, Props}}` term, found {count}")]
    MultipleApplications {
        /// How many top-level terms the file holds.
        count: usize,
    },
    /// The properties do not include `vsn`, which OTP requires.
    #[error("application `{name}` has no `vsn` property")]
    MissingVsn {
        /// The application the file names.
        name: String,
    },
    /// A list that must hold atoms holds something else.
    #[error("`{key}` must be a list of atoms, found {found}")]
    NonAtomEntry {
        /// The property holding the offending list.
        key: String,
        /// A description of the offending element.
        found: String,
    },
    /// A property's value is not of the kind that property must hold.
    #[error("`{key}` must be {expected}, found {found}")]
    InvalidValue {
        /// The property with the wrong kind of value.
        key: String,
        /// What the property must hold, as a phrase.
        expected: &'static str,
        /// A description of what it holds instead.
        found: String,
    },
}

/// The properties this module reads. Anything else is parsed and discarded.
///
/// Only these keys are watched for duplicates: a repeated key ginary never
/// reads shadows nothing, so warning about it would be noise.
const KNOWN_KEYS: [&str; 9] = [
    "vsn",
    "description",
    "applications",
    "optional_applications",
    "included_applications",
    "modules",
    "registered",
    "mod",
    "env",
];

impl TryFrom<&[Term]> for AppResource {
    type Error = AppFileError;

    /// Interprets the terms of an `.app` file.
    ///
    /// Exactly one `{application, Name, Props}` term is required, and `vsn` is
    /// the only property OTP itself insists on. A duplicate key is recoverable:
    /// the last value wins, as `file:consult/1` plus a fold would give, and the
    /// shadowing is recorded in [`AppResource::warnings`].
    ///
    /// # Errors
    ///
    /// [`AppFileError::NotAnApplication`], [`AppFileError::MultipleApplications`],
    /// [`AppFileError::MissingVsn`], [`AppFileError::NonAtomEntry`] or
    /// [`AppFileError::InvalidValue`], depending on which shape is wrong.
    fn try_from(terms: &[Term]) -> Result<Self, Self::Error> {
        let [term] = terms else {
            return Err(match terms.len() {
                0 => AppFileError::NotAnApplication {
                    found: "no terms at all".to_owned(),
                },
                count => AppFileError::MultipleApplications { count },
            });
        };

        let Term::Tuple(parts) = term else {
            return Err(AppFileError::NotAnApplication {
                found: describe_term(term),
            });
        };
        let [Term::Atom(tag), Term::Atom(name), Term::List(props)] = parts.as_slice() else {
            return Err(AppFileError::NotAnApplication {
                found: describe_term(term),
            });
        };
        if tag != "application" {
            return Err(AppFileError::NotAnApplication {
                found: describe_term(term),
            });
        }

        let mut resource = Self {
            name: name.clone(),
            vsn: String::new(),
            description: None,
            applications: Vec::new(),
            optional_applications: Vec::new(),
            included_applications: Vec::new(),
            modules: Vec::new(),
            registered: Vec::new(),
            has_mod: false,
            env_keys: Vec::new(),
            warnings: Vec::new(),
        };
        let mut vsn = None;
        let mut seen = BTreeSet::new();

        for property in props {
            let Some((key, value)) = as_pair(property) else {
                resource.warnings.push(format!(
                    "ignoring a property that is not `{{Key, Value}}`: {}",
                    describe_term(property)
                ));
                continue;
            };
            if KNOWN_KEYS.contains(&key.as_str()) && !seen.insert(key.clone()) {
                resource
                    .warnings
                    .push(format!("duplicate key `{key}`; the last value wins"));
            }
            match key.as_str() {
                "vsn" => vsn = Some(string_value("vsn", value)?),
                "description" => resource.description = Some(string_value("description", value)?),
                "applications" => resource.applications = atom_list("applications", value)?,
                "optional_applications" => {
                    resource.optional_applications = atom_list("optional_applications", value)?;
                }
                "included_applications" => {
                    resource.included_applications = atom_list("included_applications", value)?;
                }
                "modules" => resource.modules = atom_list("modules", value)?,
                "registered" => resource.registered = atom_list("registered", value)?,
                "mod" => resource.has_mod = true,
                "env" => resource.env_keys = env_keys(value, &mut resource.warnings),
                _ => {}
            }
        }

        let Some(vsn) = vsn else {
            return Err(AppFileError::MissingVsn {
                name: resource.name,
            });
        };
        resource.vsn = vsn;
        Ok(resource)
    }
}

/// Splits a `{Key, Value}` property, or `None` if it is not one.
fn as_pair(property: &Term) -> Option<(&String, &Term)> {
    let Term::Tuple(pair) = property else {
        return None;
    };
    match pair.as_slice() {
        [Term::Atom(key), value] => Some((key, value)),
        _ => None,
    }
}

/// Reads a property that must be a string.
fn string_value(key: &str, value: &Term) -> Result<String, AppFileError> {
    match value {
        Term::Str(text) => Ok(text.clone()),
        other => Err(AppFileError::InvalidValue {
            key: key.to_owned(),
            expected: "a string",
            found: describe_term(other),
        }),
    }
}

/// Reads a property that must be a list of atoms, keeping the file's order.
fn atom_list(key: &str, value: &Term) -> Result<Vec<String>, AppFileError> {
    let Term::List(items) = value else {
        return Err(AppFileError::NonAtomEntry {
            key: key.to_owned(),
            found: describe_term(value),
        });
    };
    items
        .iter()
        .map(|item| match item {
            Term::Atom(name) => Ok(name.clone()),
            other => Err(AppFileError::NonAtomEntry {
                key: key.to_owned(),
                found: describe_term(other),
            }),
        })
        .collect()
}

/// Summarises `env` by key, in file order.
///
/// The values are dropped: they can nest arbitrarily and nothing downstream
/// reads one. An entry that is not `{Key, Value}` is skipped, and the skip is
/// recorded in `warnings` rather than being silent. A repeated key is dropped
/// and warned about for the same reason a repeated property is: the second
/// value shadows the first, and a list that named the key twice would say
/// nothing about which one is live.
fn env_keys(value: &Term, warnings: &mut Vec<String>) -> Vec<String> {
    let Term::List(items) = value else {
        warnings.push(format!(
            "ignoring `env`, which must be a list, found {}",
            describe_term(value)
        ));
        return Vec::new();
    };

    let mut keys: Vec<String> = Vec::new();
    for item in items {
        match as_pair(item) {
            Some((key, _)) if keys.iter().any(|held| held == key) => {
                warnings.push(format!("duplicate `env` key `{key}`; the last value wins"))
            }
            Some((key, _)) => keys.push(key.clone()),
            None => warnings.push(format!(
                "ignoring an `env` entry that is not `{{Key, Value}}`: {}",
                describe_term(item)
            )),
        }
    }
    keys
}

/// Names a term's shape for an error message, without printing the whole tree.
fn describe_term(term: &Term) -> String {
    match term {
        Term::Atom(name) => format!("the atom `{name}`"),
        Term::Str(_) => "a string".to_owned(),
        Term::Bin(_) => "a binary".to_owned(),
        Term::Int(value) => format!("the integer `{value}`"),
        Term::Float(_) => "a float".to_owned(),
        Term::List(items) => format!("a list of {} elements", items.len()),
        Term::Tuple(items) => match items.first() {
            Some(Term::Atom(tag)) => {
                format!("a {}-tuple starting with the atom `{tag}`", items.len())
            }
            _ => format!("a {}-tuple", items.len()),
        },
    }
}

/// Reads and interprets one `.app` file.
///
/// # Errors
///
/// [`AppFileError::Io`] when the file cannot be read, [`AppFileError::Parse`]
/// when it is not Erlang term syntax, and one of the remaining variants when it
/// is Erlang but not an application resource.
pub fn parse_app_file(path: &Path) -> Result<AppResource, AppFileError> {
    let source = std::fs::read_to_string(path).map_err(|source| AppFileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let terms = parse_terms(&source).map_err(|source| AppFileError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    AppResource::try_from(terms.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_lowercase_ascii_name_is_written_without_quotes() {
        assert!(is_bare_atom("kernel"));
        assert!(is_bare_atom("gleam@crypto_ffi2"));
        assert!(!is_bare_atom(""));
        assert!(!is_bare_atom("Kernel"));
        assert!(!is_bare_atom("my-app"));
        // Accepted by the parser, still quoted on the way out.
        assert!(!is_bare_atom("\u{e9}rlang"));
        // Spelled like an atom, reserved by the grammar.
        assert!(!is_bare_atom("fun"));
        assert!(!is_bare_atom("end"));
        assert!(!is_bare_atom("maybe"));
        // Only the whole word is reserved.
        assert!(is_bare_atom("function"));
        assert!(is_bare_atom("endian"));
    }

    /// [`is_reserved_word`] binary-searches the list, so an unsorted entry
    /// would silently stop being reserved.
    #[test]
    fn the_reserved_words_are_sorted_and_all_recognised() {
        let mut sorted = RESERVED_WORDS;
        sorted.sort_unstable();
        assert_eq!(sorted, RESERVED_WORDS);
        for word in RESERVED_WORDS {
            assert!(is_reserved_word(word), "{word}");
        }
        assert!(!is_reserved_word("kernel"));
    }

    #[test]
    fn escaping_covers_the_delimiter_the_backslash_and_control_characters() {
        assert_eq!(escape("plain", '"'), "plain");
        assert_eq!(escape("a\"b", '"'), "a\\\"b");
        assert_eq!(escape("a'b", '"'), "a'b");
        assert_eq!(escape("a'b", '\''), "a\\'b");
        assert_eq!(escape("a\\b", '"'), "a\\\\b");
        assert_eq!(escape("a\nb\tc\rd", '"'), "a\\nb\\tc\\rd");
        assert_eq!(escape("\u{1}", '"'), "\\001");
    }

    /// A three-digit octal escape cannot swallow the digit that follows it.
    #[test]
    fn an_octal_escape_is_read_back_as_the_character_it_names() {
        let term = Term::Str("\u{1}7".to_owned());
        assert_eq!(term.to_string(), "\"\\0017\"");
        assert_eq!(
            parse_terms("\"\\0017\".").expect("parses"),
            vec![Term::Str("\u{1}7".to_owned())]
        );
    }

    #[test]
    fn every_float_is_written_with_a_dot_in_the_mantissa() {
        assert_eq!(render_float(1.5), "1.5");
        assert_eq!(render_float(-2000.0), "-2000.0");
        assert_eq!(render_float(0.0), "0.0");
        assert_eq!(render_float(1e-7), "1.0e-7");
        assert_eq!(render_float(1e300), "1.0e300");
        assert_eq!(render_float(f64::MIN_POSITIVE), "2.2250738585072014e-308");
    }

    #[test]
    fn a_non_finite_float_is_written_as_rust_writes_it() {
        // No Erlang literal exists, and the parser cannot produce one; the
        // point of the arm is that `Display` still cannot panic.
        assert_eq!(render_float(f64::INFINITY), "inf");
        assert_eq!(render_float(f64::NAN), "NaN");
    }

    #[test]
    fn the_largest_and_smallest_integers_survive_a_round_trip() {
        for value in [i64::MIN, i64::MAX] {
            let rendered = format!("{}.", Term::Int(value));
            assert_eq!(
                parse_terms(&rendered).expect("parses"),
                vec![Term::Int(value)]
            );
        }
    }

    #[test]
    fn an_integer_too_large_for_i64_is_an_error_and_not_a_wrapped_value() {
        let error = parse_terms("9223372036854775808.").expect_err("does not fit");
        assert_eq!((error.line, error.col), (1, 1));
        assert!(error.expected.contains("64-bit integer"), "{error}");
    }

    #[test]
    fn a_hexadecimal_escape_names_a_character() {
        assert_eq!(
            parse_terms(r#""\x41\x{1F600}"."#).expect("parses"),
            vec![Term::Str("A\u{1f600}".to_owned())]
        );
    }

    #[test]
    fn nesting_within_the_bound_parses() {
        let depth = MAX_NESTING - 1;
        let source = format!("{}{}.", "[".repeat(depth), "]".repeat(depth));
        let parsed = parse_terms(&source).expect("just inside the bound");
        assert_eq!(parsed.len(), 1);
    }

    /// A file of nothing but open brackets must produce an error, not an abort.
    #[test]
    fn nesting_past_the_bound_is_an_error_and_not_a_stack_overflow() {
        let source = "[".repeat(MAX_NESTING * 20);
        let error = parse_terms(&source).expect_err("too deep");
        assert_eq!(error.line, 1);
        assert!(error.expected.contains("levels of nesting"), "{error}");
    }

    #[test]
    fn terms_are_described_by_shape_and_not_by_their_whole_contents() {
        assert_eq!(describe_term(&Term::Atom("a".to_owned())), "the atom `a`");
        assert_eq!(describe_term(&Term::Int(-1)), "the integer `-1`");
        assert_eq!(
            describe_term(&Term::Tuple(vec![
                Term::Atom("module".to_owned()),
                Term::Atom("app".to_owned()),
            ])),
            "a 2-tuple starting with the atom `module`"
        );
        assert_eq!(
            describe_term(&Term::List(vec![Term::Int(1)])),
            "a list of 1 elements"
        );
    }

    #[test]
    fn a_property_that_is_not_a_pair_is_reported_and_skipped() {
        let terms = parse_terms("{application, app, [{vsn, \"1.0.0\"}, debug]}.").expect("parses");
        let resource = AppResource::try_from(terms.as_slice()).expect("a vsn is enough");
        assert_eq!(
            resource.warnings,
            vec!["ignoring a property that is not `{Key, Value}`: the atom `debug`".to_owned()]
        );
    }

    /// A warning names the shape of what it dropped, never the tree itself: a
    /// file whose one bad property is a huge list would otherwise echo the
    /// whole list back through `appfile parse`.
    #[test]
    fn a_warning_about_a_large_term_stays_one_short_sentence() {
        let items = vec!["x"; 1000].join(", ");
        let source = format!("{{application, app, [{{vsn, \"1.0.0\"}}, [{items}]]}}.");
        let terms = parse_terms(&source).expect("parses");
        let resource = AppResource::try_from(terms.as_slice()).expect("a vsn is enough");
        assert_eq!(
            resource.warnings,
            vec![
                "ignoring a property that is not `{Key, Value}`: a list of 1000 elements"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn an_env_entry_that_is_not_a_pair_is_reported_and_skipped() {
        let terms = parse_terms("{application, app, [{vsn, \"1.0.0\"}, {env, [{a, 1}, b]}]}.")
            .expect("parses");
        let resource = AppResource::try_from(terms.as_slice()).expect("env is recoverable");
        assert_eq!(resource.env_keys, vec!["a".to_owned()]);
        assert_eq!(
            resource.warnings,
            vec!["ignoring an `env` entry that is not `{Key, Value}`: the atom `b`".to_owned()]
        );
    }

    #[test]
    fn a_duplicate_env_key_is_dropped_and_recorded() {
        let terms =
            parse_terms("{application, app, [{vsn, \"1.0.0\"}, {env, [{a, 1}, {b, 2}, {a, 3}]}]}.")
                .expect("parses");
        let resource = AppResource::try_from(terms.as_slice()).expect("env is recoverable");
        assert_eq!(resource.env_keys, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(
            resource.warnings,
            vec!["duplicate `env` key `a`; the last value wins".to_owned()]
        );
    }

    #[test]
    fn a_vsn_that_is_not_a_string_is_an_error_naming_the_key() {
        let terms = parse_terms("{application, app, [{vsn, 1}]}.").expect("parses");
        let error = AppResource::try_from(terms.as_slice()).expect_err("vsn must be a string");
        assert!(
            matches!(&error, AppFileError::InvalidValue { key, .. } if key == "vsn"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "`vsn` must be a string, found the integer `1`"
        );
    }

    #[test]
    fn a_property_list_that_is_not_a_list_is_reported_by_key() {
        let terms =
            parse_terms("{application, app, [{vsn, \"1.0\"}, {modules, ok}]}.").expect("parses");
        let error = AppResource::try_from(terms.as_slice()).expect_err("modules must be a list");
        assert_eq!(
            error.to_string(),
            "`modules` must be a list of atoms, found the atom `ok`"
        );
    }

    /// A repeated key ginary never reads shadows nothing, so it is not a
    /// warning; the two it does read are.
    #[test]
    fn only_the_keys_ginary_reads_are_watched_for_duplicates() {
        let terms = parse_terms(
            "{application, app, [{vsn, \"1.0\"}, {runtime_dependencies, []}, \
             {runtime_dependencies, []}]}.",
        )
        .expect("parses");
        let resource = AppResource::try_from(terms.as_slice()).expect("duplicates are recoverable");
        assert!(resource.warnings.is_empty(), "{:?}", resource.warnings);
    }
}
