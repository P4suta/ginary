// SPDX-License-Identifier: MIT OR Apache-2.0
//! `Display for Term` wrote a reserved word as a bare atom.
//!
//! **What went wrong.** `is_bare_atom` quoted an atom only when it was not a
//! lowercase ASCII identifier, so `Term::Atom("fun")` rendered as `fun` and
//! `Term::Atom("end")` as `end`. Neither is an atom in Erlang: they are
//! reserved words. `parse_terms` rejected `fun.` outright, which made the
//! module's documented invariant — `Display` is the inverse of parsing — false
//! for its own parser, and made the round-trip property test a latent flake,
//! because its atom generator can produce `fun`.
//!
//! **The input.** Any term holding an atom that is an Erlang reserved word;
//! `'fun'` in a real `.app` file parses to exactly that.
//!
//! **The correct behaviour.** A reserved word is quoted on the way out, so
//! every reserved word round-trips through `parse_terms` and the output is
//! valid Erlang for a compiler too.

use ginary::appfile::{Term, parse_terms};

/// Every reserved word of the Erlang grammar, including the `maybe` feature's.
///
/// Kept as a list rather than as a sample: quoting one and forgetting another
/// is exactly the bug, so the test has to name them all.
const RESERVED: [&str; 29] = [
    "after", "and", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor", "case",
    "catch", "cond", "div", "else", "end", "fun", "if", "let", "maybe", "not", "of", "or",
    "orelse", "receive", "rem", "try", "when", "xor",
];

#[test]
fn a_reserved_word_atom_survives_display_and_a_second_parse() {
    for word in RESERVED {
        let original = Term::Atom(word.to_owned());
        let rendered = format!("{original}.");
        match parse_terms(&rendered) {
            Ok(terms) => assert_eq!(
                terms,
                vec![original],
                "`{rendered}` re-read as something else"
            ),
            Err(error) => panic!("`{rendered}` did not parse back: {error}"),
        }
    }
}

#[test]
fn a_reserved_word_is_quoted_inside_a_nested_term() {
    let original = Term::Tuple(vec![
        Term::Atom("mod".to_owned()),
        Term::List(vec![
            Term::Atom("fun".to_owned()),
            Term::Atom("kernel".to_owned()),
        ]),
    ]);
    assert_eq!(original.to_string(), "{mod, ['fun', kernel]}");
    assert_eq!(
        parse_terms(&format!("{original}.")).expect("the rendering parses back"),
        vec![original]
    );
}
