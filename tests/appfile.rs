// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `.app` reader: the term grammar, the resource it produces, and the
//! errors it produces instead.
//!
//! Three layers are covered here and each is asserted on its own, because they
//! fail for different reasons and a test that mixed them would not say which
//! broke: [`parse_terms`] over source text, [`AppResource`] over terms, and
//! [`parse_app_file`] over a file on disk.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use crate::common::hostpath::strip_dir;

use std::path::{Path, PathBuf};

use ginary::appfile::{AppFileError, AppResource, ParseError, Term, parse_app_file, parse_terms};
use proptest::prelude::*;

use crate::common::fake_otp::{DUMMY_BEAM, FakeApp, FakeOtp, FakeShipment};
use crate::common::tools::require_tools;

/// The directory the hand-written and copied fixtures live in.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app")
}

/// One fixture by relative path, for example `otp/kernel.app`.
fn fixture(relative: &str) -> PathBuf {
    fixtures().join(relative)
}

/// Parses `src` or fails the test with the error.
fn terms(src: &str) -> Vec<Term> {
    match parse_terms(src) {
        Ok(terms) => terms,
        Err(error) => panic!("`{src}` should parse, but: {error}"),
    }
}

/// Parses `src` and returns the single term it holds.
fn term(src: &str) -> Term {
    let mut parsed = terms(src);
    assert_eq!(parsed.len(), 1, "`{src}` should hold exactly one term");
    parsed.remove(0)
}

/// The error from parsing `src`, which must fail.
fn parse_failure(src: &str) -> ParseError {
    match parse_terms(src) {
        Ok(terms) => panic!("`{src}` should not parse, but it produced {terms:?}"),
        Err(error) => error,
    }
}

/// Reads a fixture or fails the test with the error.
fn resource(relative: &str) -> AppResource {
    let path = fixture(relative);
    match parse_app_file(&path) {
        Ok(resource) => resource,
        Err(error) => panic!("{relative} should parse, but: {error}"),
    }
}

/// The error from reading a fixture, which must fail.
fn app_failure(relative: &str) -> AppFileError {
    let path = fixture(relative);
    match parse_app_file(&path) {
        Ok(resource) => panic!("{relative} should not parse, but it produced {resource:?}"),
        Err(error) => error,
    }
}

/// Owns a list of borrowed names, for comparing against parsed fields.
fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// ---------------------------------------------------------------- parse_terms

#[test]
fn an_empty_source_holds_no_terms() {
    assert_eq!(terms(""), Vec::<Term>::new());
    assert_eq!(terms("   \n\t\n"), Vec::<Term>::new());
}

#[test]
fn a_source_of_only_comments_holds_no_terms() {
    assert_eq!(terms("% one\n%% two\n   % three"), Vec::<Term>::new());
}

#[test]
fn bare_atoms_parse_as_atoms() {
    assert_eq!(term("kernel."), Term::Atom("kernel".to_owned()));
    assert_eq!(
        term("gleam@crypto_ffi2."),
        Term::Atom("gleam@crypto_ffi2".to_owned())
    );
}

#[test]
fn quoted_atoms_keep_their_unquoted_name_and_escapes() {
    assert_eq!(term("'my-app'."), Term::Atom("my-app".to_owned()));
    assert_eq!(term(r"'it\'s'."), Term::Atom("it's".to_owned()));
    assert_eq!(term(r"'a\\b'."), Term::Atom(r"a\b".to_owned()));
    assert_eq!(term(r"'a\nb\tc'."), Term::Atom("a\nb\tc".to_owned()));
}

#[test]
fn strings_unescape_quotes_backslashes_and_control_characters() {
    assert_eq!(
        term(r#""a \"quoted\" app"."#),
        Term::Str("a \"quoted\" app".to_owned())
    );
    assert_eq!(
        term(r#""back\\slash"."#),
        Term::Str(r"back\slash".to_owned())
    );
    assert_eq!(
        term(r#""line\ntab\t"."#),
        Term::Str("line\ntab\t".to_owned())
    );
}

#[test]
fn a_percent_inside_a_string_is_not_a_comment() {
    assert_eq!(
        term(r#""100% not a comment"."#),
        Term::Str("100% not a comment".to_owned())
    );
}

#[test]
fn binaries_parse_with_and_without_contents() {
    assert_eq!(term(r#"<<"payload">>."#), Term::Bin("payload".to_owned()));
    assert_eq!(term("<<>>."), Term::Bin(String::new()));
}

#[test]
fn integers_accept_a_leading_minus() {
    assert_eq!(term("42."), Term::Int(42));
    assert_eq!(term("0."), Term::Int(0));
    assert_eq!(term("-1."), Term::Int(-1));
}

#[test]
fn floats_accept_a_sign_and_an_exponent() {
    assert_eq!(term("1.5."), Term::Float(1.5));
    assert_eq!(term("-2.0e3."), Term::Float(-2000.0));
    assert_eq!(term("0.25."), Term::Float(0.25));
}

#[test]
fn character_literals_are_integers() {
    assert_eq!(term("$a."), Term::Int(97));
    assert_eq!(term(r"$\n."), Term::Int(10));
    assert_eq!(term(r"$\\."), Term::Int(92));
    assert_eq!(term(r"$\t."), Term::Int(9));
}

#[test]
fn tuples_and_lists_nest_and_may_be_empty() {
    assert_eq!(term("{}."), Term::Tuple(Vec::new()));
    assert_eq!(term("[]."), Term::List(Vec::new()));
    assert_eq!(
        term("{a, [b, {c, []}]}."),
        Term::Tuple(vec![
            Term::Atom("a".to_owned()),
            Term::List(vec![
                Term::Atom("b".to_owned()),
                Term::Tuple(vec![Term::Atom("c".to_owned()), Term::List(Vec::new())]),
            ]),
        ])
    );
}

#[test]
fn several_top_level_terms_are_returned_in_order() {
    assert_eq!(
        terms("a.\nb.\n% comment\nc."),
        vec![
            Term::Atom("a".to_owned()),
            Term::Atom("b".to_owned()),
            Term::Atom("c".to_owned()),
        ]
    );
}

#[test]
fn a_term_without_a_final_full_stop_is_an_error() {
    let error = parse_failure("{a, b}");
    assert_eq!((error.line, error.col), (1, 7));
    assert_eq!(error.expected, "`.`");
    assert_eq!(error.found, "end of input");
}

/// Erlang's own rule: a term ends with a full stop *followed by whitespace*,
/// which is what keeps `1.5` from being an integer and an empty term. Without
/// it `a.b.` would be read as two terms rather than as the typo it is.
#[test]
fn a_full_stop_must_be_followed_by_whitespace_or_the_end_of_input() {
    let error = parse_failure("a.b.");
    assert_eq!((error.line, error.col), (1, 2));
    assert!(error.expected.contains("followed by whitespace"), "{error}");
    assert_eq!(error.found, "`.`");
}

#[test]
fn a_full_stop_closes_a_term_at_end_of_input_before_a_comment_and_before_a_newline() {
    assert_eq!(terms("a."), vec![Term::Atom("a".to_owned())]);
    assert_eq!(terms("a.% trailing"), vec![Term::Atom("a".to_owned())]);
    assert_eq!(
        terms(
            "a.
b."
        ),
        vec![Term::Atom("a".to_owned()), Term::Atom("b".to_owned())]
    );
}

#[test]
fn a_binary_holds_bytes_and_a_wider_character_is_rejected() {
    assert_eq!(
        term("<<\"\u{e9}\">>."),
        Term::Bin("\u{e9}".to_owned()),
        "233 is a byte, so the binary is a binary"
    );
    let error = parse_failure("<<\"\u{1f600}\">>.");
    assert_eq!((error.line, error.col), (1, 3));
    assert!(error.expected.contains("up to 255"), "{error}");
    assert!(error.found.contains("128512"), "{error}");
}

#[test]
fn a_list_tail_is_rejected_by_name() {
    let error = parse_failure("[a | b].");
    assert_eq!((error.line, error.col), (1, 4));
    assert_eq!(error.found, "a list tail (`|`)");
}

#[test]
fn a_map_is_rejected_by_name() {
    let error = parse_failure("#{a => 1}.");
    assert_eq!((error.line, error.col), (1, 1));
    assert_eq!(error.expected, "a term");
    assert_eq!(error.found, "a map (`#{`)");
}

#[test]
fn a_fun_is_rejected_by_name() {
    let error = parse_failure("fun erlang:halt/1.");
    assert_eq!((error.line, error.col), (1, 1));
    assert_eq!(error.found, "a fun (`fun`)");
}

#[test]
fn a_variable_is_rejected_by_name() {
    let error = parse_failure("[Value].");
    assert_eq!((error.line, error.col), (1, 2));
    assert_eq!(error.found, "a variable (`Value`)");
}

#[test]
fn an_unterminated_string_is_an_error() {
    let error = parse_failure("\"open");
    assert_eq!((error.line, error.col), (1, 1));
    assert_eq!(error.found, "an unterminated string");
}

#[test]
fn the_error_position_counts_lines_and_characters() {
    let error = parse_failure("[a,\n b,\n  é#{}].");
    assert_eq!(
        (error.line, error.col),
        (3, 4),
        "the column counts characters, not bytes: {error}"
    );
}

// -------------------------------------------------------------- Term::Display

#[test]
fn display_quotes_an_atom_only_when_it_has_to() {
    assert_eq!(Term::Atom("kernel".to_owned()).to_string(), "kernel");
    assert_eq!(
        Term::Atom("gleam@crypto".to_owned()).to_string(),
        "gleam@crypto"
    );
    assert_eq!(Term::Atom("my-app".to_owned()).to_string(), "'my-app'");
    assert_eq!(Term::Atom("Value".to_owned()).to_string(), "'Value'");
    assert_eq!(Term::Atom(String::new()).to_string(), "''");
    assert_eq!(Term::Atom("it's".to_owned()).to_string(), r"'it\'s'");
    assert_eq!(Term::Atom(r"a\b".to_owned()).to_string(), r"'a\\b'");
}

#[test]
fn display_escapes_strings_and_binaries() {
    assert_eq!(Term::Str("plain".to_owned()).to_string(), "\"plain\"");
    assert_eq!(
        Term::Str("say \"hi\"".to_owned()).to_string(),
        r#""say \"hi\"""#
    );
    assert_eq!(Term::Str("a\nb\t".to_owned()).to_string(), r#""a\nb\t""#);
    assert_eq!(
        Term::Bin("payload".to_owned()).to_string(),
        r#"<<"payload">>"#
    );
    assert_eq!(Term::Bin(String::new()).to_string(), "<<>>");
}

#[test]
fn display_separates_elements_with_a_comma_and_a_space() {
    assert_eq!(
        Term::Tuple(vec![
            Term::Atom("a".to_owned()),
            Term::List(vec![Term::Int(-1), Term::Float(1.5)]),
            Term::Tuple(Vec::new()),
        ])
        .to_string(),
        "{a, [-1, 1.5], {}}"
    );
}

#[test]
fn display_always_writes_a_float_erlang_can_read_back() {
    assert_eq!(Term::Float(1.5).to_string(), "1.5");
    assert_eq!(Term::Float(-2000.0).to_string(), "-2000.0");
    assert_eq!(Term::Float(0.0).to_string(), "0.0");
    assert_eq!(Term::Float(1e-7).to_string(), "1.0e-7");
    assert_eq!(Term::Float(1e300).to_string(), "1.0e300");
}

/// Any term the parser can produce must survive `Display` and a second parse.
///
/// The generator is capped at depth 4 because the property is about the
/// grammar, not about recursion: a deeper tree costs time without reaching a
/// case a depth-4 tree does not already contain.
fn any_term() -> impl Strategy<Value = Term> {
    let atom = prop_oneof![
        "[a-z][a-zA-Z0-9_@]{0,5}",
        "[A-Za-z0-9_@ '\\\\-]{0,6}",
        Just("kernel".to_owned()),
        // Reserved words are spelled like atoms and are not atoms; the first
        // strategy can produce them, but only about once in a million cases.
        Just("fun".to_owned()),
        Just("end".to_owned()),
        Just("maybe".to_owned()),
    ];
    let text = prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just('Z'),
            Just('9'),
            Just(' '),
            Just('"'),
            Just('\\'),
            Just('\n'),
            Just('\t'),
            Just('%'),
            Just('\u{e9}'),
        ],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect::<String>());

    let leaf = prop_oneof![
        atom.prop_map(Term::Atom),
        text.clone().prop_map(Term::Str),
        text.prop_map(Term::Bin),
        any::<i64>().prop_map(Term::Int),
        (-1.0e9f64..1.0e9f64).prop_map(Term::Float),
    ];

    leaf.prop_recursive(4, 48, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Term::Tuple),
            prop::collection::vec(inner, 0..4).prop_map(Term::List),
        ]
    })
}

proptest! {
    #[test]
    fn display_round_trips_through_parse_terms(original in any_term()) {
        let rendered = format!("{original}.");
        let reparsed = parse_terms(&rendered)
            .map_err(|error| TestCaseError::fail(format!("`{rendered}` did not parse: {error}")))?;
        prop_assert_eq!(reparsed, vec![original]);
    }
}

#[test]
fn the_nested_fixture_re_serialises_to_one_line() {
    let source = std::fs::read_to_string(fixture("nested.app")).expect("the fixture is readable");
    let rendered: String = terms(&source)
        .iter()
        .map(|term| format!("{term}."))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("nested_term_display", rendered);
}

// ------------------------------------------------------- AppResource from Term

/// `{application, Name, Props}` around the given properties.
fn application(name: &str, props: Vec<Term>) -> Vec<Term> {
    vec![Term::Tuple(vec![
        Term::Atom("application".to_owned()),
        Term::Atom(name.to_owned()),
        Term::List(props),
    ])]
}

/// `{Key, Value}`.
fn prop(key: &str, value: Term) -> Term {
    Term::Tuple(vec![Term::Atom(key.to_owned()), value])
}

/// A list of atoms.
fn atoms(values: &[&str]) -> Term {
    Term::List(
        values
            .iter()
            .map(|value| Term::Atom((*value).to_owned()))
            .collect(),
    )
}

#[test]
fn a_minimal_application_term_yields_a_resource() {
    let source = application("app", vec![prop("vsn", Term::Str("1.0.0".to_owned()))]);
    let resource = AppResource::try_from(source.as_slice()).expect("a vsn is all that is required");

    assert_eq!(resource.name, "app");
    assert_eq!(resource.vsn, "1.0.0");
    assert_eq!(resource.description, None);
    assert!(resource.applications.is_empty());
    assert!(resource.included_applications.is_empty());
    assert!(resource.modules.is_empty());
    assert!(resource.registered.is_empty());
    assert!(!resource.has_mod);
    assert!(resource.env_keys.is_empty());
    assert!(resource.warnings.is_empty());
}

#[test]
fn an_application_without_a_vsn_is_an_error() {
    let source = application("app", vec![prop("description", Term::Str("x".to_owned()))]);
    let error = AppResource::try_from(source.as_slice()).expect_err("vsn is required");
    assert!(
        matches!(&error, AppFileError::MissingVsn { name } if name == "app"),
        "{error:?}"
    );
}

#[test]
fn a_duplicate_key_keeps_the_last_value_and_records_a_warning() {
    let source = application(
        "app",
        vec![
            prop("vsn", Term::Str("1.0.0".to_owned())),
            prop("applications", atoms(&["kernel"])),
            prop("vsn", Term::Str("2.0.0".to_owned())),
            prop("applications", atoms(&["stdlib"])),
        ],
    );
    let resource = AppResource::try_from(source.as_slice()).expect("duplicates are recoverable");

    assert_eq!(resource.vsn, "2.0.0");
    assert_eq!(resource.applications, names(&["stdlib"]));
    assert_eq!(
        resource.warnings,
        vec![
            "duplicate key `vsn`; the last value wins".to_owned(),
            "duplicate key `applications`; the last value wins".to_owned(),
        ]
    );
}

#[test]
fn a_non_atom_in_applications_is_an_error() {
    let source = application(
        "app",
        vec![
            prop("vsn", Term::Str("1.0.0".to_owned())),
            prop(
                "applications",
                Term::List(vec![
                    Term::Atom("kernel".to_owned()),
                    Term::Str("stdlib".to_owned()),
                ]),
            ),
        ],
    );
    let error = AppResource::try_from(source.as_slice()).expect_err("only atoms name applications");
    assert!(
        matches!(&error, AppFileError::NonAtomEntry { key, .. } if key == "applications"),
        "{error:?}"
    );
}

#[test]
fn more_than_one_top_level_term_is_an_error() {
    let mut source = application("app", vec![prop("vsn", Term::Str("1.0.0".to_owned()))]);
    source.push(Term::Atom("extra".to_owned()));
    let error = AppResource::try_from(source.as_slice()).expect_err("an .app file holds one term");
    assert!(
        matches!(error, AppFileError::MultipleApplications { count: 2 }),
        "{error:?}"
    );
}

#[test]
fn a_term_that_is_not_an_application_is_an_error() {
    let source = vec![Term::Tuple(vec![
        Term::Atom("module".to_owned()),
        Term::Atom("app".to_owned()),
    ])];
    let error =
        AppResource::try_from(source.as_slice()).expect_err("only `application` terms are read");
    assert!(
        matches!(error, AppFileError::NotAnApplication { .. }),
        "{error:?}"
    );
}

#[test]
fn no_terms_at_all_is_an_error() {
    let error = AppResource::try_from([].as_slice()).expect_err("an empty file has no application");
    assert!(
        matches!(error, AppFileError::NotAnApplication { .. }),
        "{error:?}"
    );
}

// -------------------------------------------------- hand-written app fixtures

#[test]
fn quoted_names_survive_into_the_resource() {
    let resource = resource("quoted.app");
    assert_eq!(resource.name, "my-app");
    assert_eq!(resource.vsn, "0.1.0");
    assert_eq!(resource.description.as_deref(), Some("a \"quoted\" app"));
    assert_eq!(resource.modules, names(&["my-app", "my-app@sup"]));
    assert_eq!(resource.registered, names(&["my-app_sup"]));
    assert_eq!(resource.applications, names(&["kernel", "stdlib"]));
    assert!(
        resource.has_mod,
        "`{{mod, {{'x@y', []}}}}` is a mod property"
    );
    assert_eq!(resource.env_keys, names(&["weird key"]));
    assert!(resource.warnings.is_empty(), "{:?}", resource.warnings);
}

#[test]
fn comments_never_reach_the_resource() {
    let resource = resource("comments.app");
    assert_eq!(resource.name, "comments");
    assert_eq!(resource.vsn, "1.0.0");
    assert_eq!(
        resource.description.as_deref(),
        Some("100% not a comment"),
        "a `%` inside a string is data, not a comment"
    );
    assert_eq!(resource.applications, names(&["kernel", "stdlib"]));
    assert!(resource.modules.is_empty());
}

#[test]
fn included_applications_stay_separate_from_applications() {
    let resource = resource("included.app");
    assert_eq!(resource.vsn, "2.5.0");
    assert_eq!(
        resource.applications,
        names(&["kernel", "stdlib", "crypto"])
    );
    assert_eq!(
        resource.included_applications,
        names(&["sasl", "runtime_tools"])
    );
    assert_eq!(resource.registered, names(&["included_sup"]));
    assert!(!resource.has_mod);
}

#[test]
fn nested_env_values_are_summarised_by_key_in_file_order() {
    let resource = resource("nested.app");
    assert_eq!(resource.name, "nested");
    assert_eq!(resource.vsn, "0.3.1");
    assert_eq!(
        resource.env_keys,
        names(&[
            "bin",
            "empty_bin",
            "chars",
            "floats",
            "ints",
            "tree",
            "unit"
        ])
    );
    assert_eq!(resource.modules, names(&["nested", "nested_ffi"]));
    assert!(!resource.has_mod);
}

// --------------------------------------------------------------- fake builders

/// The builders in `tests/common/fake_otp.rs` write `.app` files that later
/// milestones read back through this parser, so the two are checked against
/// each other here rather than in the first test that trips over a difference.
#[test]
fn a_fake_shipment_writes_an_app_file_this_parser_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let shipment = FakeShipment::new()
        .app("gleam_stdlib", "0.65.0", &["kernel", "stdlib"])
        .app_with("notify", "1.2.3", |app| {
            app.description("a notifier")
                .applications(&["kernel", "stdlib", "gleam_stdlib"])
                .included(&["sasl"])
                .modules(&["notify", "notify@sup"])
                .registered(&["notify_sup"])
                .mod_callback("notify_app")
                .env("port", "8080")
                .env("host", "\"localhost\"")
                .priv_file("static/index.html", b"<!doctype html>")
        })
        .build_in(dir.path());

    let app_file = shipment.app_file("notify");
    let resource = match parse_app_file(&app_file) {
        Ok(resource) => resource,
        Err(error) => panic!("{} should parse, but: {error}", app_file.display()),
    };

    assert_eq!(resource.name, "notify");
    assert_eq!(resource.vsn, "1.2.3");
    assert_eq!(resource.description.as_deref(), Some("a notifier"));
    assert_eq!(
        resource.applications,
        names(&["kernel", "stdlib", "gleam_stdlib"])
    );
    assert_eq!(resource.included_applications, names(&["sasl"]));
    assert_eq!(resource.modules, names(&["notify", "notify@sup"]));
    assert_eq!(resource.registered, names(&["notify_sup"]));
    assert!(resource.has_mod);
    assert_eq!(resource.env_keys, names(&["port", "host"]));
    assert!(resource.warnings.is_empty(), "{:?}", resource.warnings);

    let ebin = shipment.app_dir("notify").join("ebin");
    assert_eq!(
        std::fs::read(ebin.join("notify@sup.beam")).expect("the dummy beam is written"),
        DUMMY_BEAM
    );
    assert_eq!(
        std::fs::read(
            shipment
                .app_dir("notify")
                .join("priv")
                .join("static/index.html")
        )
        .expect("the priv file is written"),
        b"<!doctype html>"
    );
    assert!(shipment.app_file("gleam_stdlib").is_file());
}

/// OTP itself ships `my-app`-shaped names, and a hyphen makes a quoted atom.
/// A builder that wrote one bare would produce a file this parser rejects.
#[test]
fn a_fake_app_whose_name_is_not_a_bare_atom_is_still_written_as_erlang() {
    let app = FakeApp::new("my-app", "0.1.0")
        .modules(&["my-app", "my-app@sup"])
        .registered(&["my-app_sup"])
        .mod_callback("my-app");
    assert_eq!(app.name(), "my-app");
    assert_eq!(app.vsn(), "0.1.0");

    let parsed = terms(&app.app_text());
    let resource = match AppResource::try_from(parsed.as_slice()) {
        Ok(resource) => resource,
        Err(error) => panic!("the builder should write a resource, but: {error}"),
    };
    assert_eq!(resource.name, "my-app");
    assert_eq!(resource.modules, names(&["my-app", "my-app@sup"]));
    assert!(resource.has_mod);
}

#[test]
fn a_fake_otp_root_writes_app_files_this_parser_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .app_with("ssl", "11.7.4", |app| {
            app.applications(&["crypto", "public_key"])
                .priv_file("lib/x.so", b"..")
        })
        .build_in(dir.path());

    let app_file = otp.app_dir("ssl").join("ebin").join("ssl.app");
    let resource = match parse_app_file(&app_file) {
        Ok(resource) => resource,
        Err(error) => panic!("{} should parse, but: {error}", app_file.display()),
    };
    assert_eq!(resource.name, "ssl");
    assert_eq!(resource.vsn, "11.7.4");
    assert_eq!(resource.applications, names(&["crypto", "public_key"]));

    let kernel = otp.app_dir("kernel").join("ebin").join("kernel.app");
    let kernel = match parse_app_file(&kernel) {
        Ok(resource) => resource,
        Err(error) => panic!("the seeded kernel should parse, but: {error}"),
    };
    assert!(kernel.has_mod, "the seed carries a `mod` callback");
}

#[test]
fn optional_applications_are_read_and_kept_apart_from_applications() {
    let source = "{application, opt, [{vsn, \"1.0.0\"},\n\
                  {applications, [kernel, stdlib, observer]},\n\
                  {optional_applications, [observer]}]}.\n";
    let parsed = terms(source);
    let resource = match AppResource::try_from(parsed.as_slice()) {
        Ok(resource) => resource,
        Err(error) => panic!("the source should describe an application, but: {error}"),
    };
    assert_eq!(
        resource.applications,
        names(&["kernel", "stdlib", "observer"])
    );
    assert_eq!(resource.optional_applications, names(&["observer"]));
    assert!(resource.warnings.is_empty(), "{:?}", resource.warnings);
}

#[test]
fn a_duplicate_optional_applications_key_is_warned_about_like_any_other() {
    let source = "{application, opt, [{vsn, \"1.0.0\"},\n\
                  {optional_applications, [a]},\n\
                  {optional_applications, [b]}]}.\n";
    let parsed = terms(source);
    let resource = match AppResource::try_from(parsed.as_slice()) {
        Ok(resource) => resource,
        Err(error) => panic!("the source should describe an application, but: {error}"),
    };
    assert_eq!(resource.optional_applications, names(&["b"]));
    assert_eq!(
        resource.warnings,
        vec!["duplicate key `optional_applications`; the last value wins".to_owned()]
    );
}

// ------------------------------------------------------------------- failures

#[test]
fn the_malformed_fixture_reports_line_five_column_three() {
    let error = app_failure("malformed.app");
    let AppFileError::Parse { path, source } = &error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert_eq!(path, &fixture("malformed.app"));
    assert_eq!((source.line, source.col), (5, 3));
    assert_eq!(source.expected, "`,` or `}`");
    assert_eq!(source.found, "`{`");
}

#[test]
fn the_unsupported_map_fixture_names_the_construct() {
    let error = app_failure("unsupported_map.app");
    let AppFileError::Parse { source, .. } = &error else {
        panic!("expected a parse error, got {error:?}");
    };
    assert_eq!((source.line, source.col), (7, 21));
    assert_eq!(source.expected, "a term");
    assert_eq!(source.found, "a map (`#{`)");
}

#[test]
fn a_missing_file_is_an_io_error_naming_the_path() {
    let path = fixture("does_not_exist.app");
    let error = parse_app_file(&path).expect_err("the file is not there");
    let AppFileError::Io { path: reported, .. } = &error else {
        panic!("expected an I/O error, got {error:?}");
    };
    assert_eq!(reported, &path);
}

#[test]
fn the_failure_messages_read_as_sentences() {
    let rendered = [
        app_failure("malformed.app"),
        app_failure("unsupported_map.app"),
    ]
    .iter()
    .map(|error| {
        // The fixture directory is absolute and machine-specific; the file name
        // is the part the message has to get right. Stripped through
        // `hostpath::strip_dir` because `Path::join` uses the host's separator
        // and gluing a `/` onto the directory matches nothing on a machine
        // that joined with `\`.
        strip_dir(&error.to_string(), &fixtures())
    })
    .collect::<Vec<_>>()
    .join("\n");
    insta::assert_snapshot!("parse_error_messages", rendered);
}

// --------------------------------------------------------- copied real files

#[test]
fn the_copied_otp_fixtures_parse_with_the_versions_they_shipped_with() {
    let kernel = resource("otp/kernel.app");
    assert_eq!(kernel.name, "kernel");
    assert_eq!(kernel.vsn, "11.0.3");
    assert_eq!(kernel.description.as_deref(), Some("ERTS  CXC 138 10"));
    assert!(kernel.applications.is_empty(), "{:?}", kernel.applications);
    assert!(kernel.has_mod);
    assert_eq!(
        kernel.env_keys,
        names(&[
            "logger_level",
            "logger_sasl_compatible",
            "net_tickintensity",
            "net_ticktime",
            "prevent_overlapping_partitions",
            "shell_docs_ansi",
            "shell_history_drop",
        ])
    );

    let stdlib = resource("otp/stdlib.app");
    assert_eq!(stdlib.vsn, "8.0.3");
    assert_eq!(stdlib.applications, names(&["kernel"]));
    assert!(!stdlib.has_mod);
    assert!(stdlib.env_keys.is_empty());

    let ssl = resource("otp/ssl.app");
    assert_eq!(ssl.vsn, "11.7.4");
    assert_eq!(
        ssl.applications,
        names(&["crypto", "public_key", "kernel", "stdlib"])
    );
    assert_eq!(ssl.registered, names(&["ssl_sup", "ssl_manager"]));
    assert!(ssl.has_mod);

    let inets = resource("otp/inets.app");
    assert_eq!(inets.vsn, "9.7.1");
    assert_eq!(inets.applications, names(&["kernel", "stdlib"]));
    assert_eq!(inets.registered, names(&["inets_sup", "httpc_manager"]));
    assert!(inets.has_mod);

    let crypto = resource("otp/crypto.app");
    assert_eq!(crypto.vsn, "5.9.2");
    assert_eq!(crypto.applications, names(&["kernel", "stdlib"]));
    assert_eq!(crypto.env_keys, names(&["fips_mode", "rand_cache_size"]));
    assert!(!crypto.has_mod);
    assert!(crypto.registered.is_empty());
}

#[test]
fn the_copied_shipment_fixtures_parse_as_gleam_wrote_them() {
    let crypto = resource("shipment/gleam_crypto.app");
    assert_eq!(crypto.name, "gleam_crypto");
    assert_eq!(crypto.vsn, "1.6.0");
    assert_eq!(crypto.applications, names(&["crypto", "gleam_stdlib"]));
    assert_eq!(crypto.modules, names(&["gleam@crypto", "gleam_crypto_ffi"]));
    assert_eq!(
        crypto.description.as_deref(),
        Some("A Gleam cryptography library supporting Erlang and JavaScript")
    );
    assert!(!crypto.has_mod);
    assert!(crypto.env_keys.is_empty());

    let mist = resource("shipment/mist.app");
    assert_eq!(mist.vsn, "6.0.3");
    assert!(
        mist.has_mod,
        "mist declares {{mod, {{'mist@internal@clock', []}}}}"
    );
    assert_eq!(
        mist.applications,
        names(&[
            "exception",
            "gleam_erlang",
            "gleam_http",
            "gleam_otp",
            "gleam_stdlib",
            "glisten",
            "gramps",
            "hpack",
            "logging",
        ])
    );

    let stdlib = resource("shipment/gleam_stdlib.app");
    assert_eq!(stdlib.vsn, "1.0.5");
    assert!(stdlib.applications.is_empty());
    assert_eq!(stdlib.modules.len(), 20, "{:?}", stdlib.modules);
    assert_eq!(
        stdlib.modules.first().map(String::as_str),
        Some("gleam@bit_array")
    );

    let notify = resource("shipment/notify.app");
    assert_eq!(notify.name, "notify");
    assert_eq!(notify.vsn, "0.1.0");
    assert_eq!(notify.applications.len(), 13, "{:?}", notify.applications);
    assert_eq!(
        notify.applications.first().map(String::as_str),
        Some("argus")
    );
    assert!(!notify.has_mod);
}

/// Every `.app` in the OTP installation on this machine, not just the copies.
///
/// The copied fixtures pin behaviour on any machine; this one pins that the
/// grammar is complete for a whole real installation, including the
/// applications ginary will never package. Gated on `erl`, because without it
/// there is no installation to walk.
#[test]
fn parses_every_app_in_host_otp() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };

    let info = match ginary::otp::discover(None) {
        Ok(info) => info,
        Err(error) => panic!("`erl` is on PATH but discovery failed: {error}"),
    };

    let mut checked = 0_usize;
    let mut failures = Vec::new();
    let lib = std::fs::read_dir(&info.lib).expect("the OTP lib directory is readable");
    for entry in lib {
        let ebin = entry.expect("a readable lib entry").path().join("ebin");
        let Ok(files) = std::fs::read_dir(&ebin) else {
            continue;
        };
        for file in files {
            let path = file.expect("a readable ebin entry").path();
            if path.extension().is_none_or(|extension| extension != "app") {
                continue;
            }
            checked += 1;
            if let Err(error) = parse_app_file(&path) {
                failures.push(format!("{}: {error}", path.display()));
            }
        }
    }

    assert!(
        checked >= 20,
        "only {checked} .app files found under {}",
        info.lib.display()
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} .app files failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
