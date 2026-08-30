// SPDX-License-Identifier: MIT OR Apache-2.0
//! The regression suite: one module per fixed bug.
//!
//! Cargo compiles only the `.rs` files directly under `tests/` as test targets,
//! so the files in `tests/regressions/` are included from here. See
//! `tests/regressions/README.md` for the convention each of them follows.

mod common;

#[path = "regressions/a1a_display_left_reserved_words_bare.rs"]
mod a1a_display_left_reserved_words_bare;
#[path = "regressions/a1a_doctor_dropped_the_otp_error.rs"]
mod a1a_doctor_dropped_the_otp_error;
#[path = "regressions/a1a_env_duplicate_keys_were_unreported.rs"]
mod a1a_env_duplicate_keys_were_unreported;
#[path = "regressions/a1b_an_ambiguous_optional_edge_was_an_error.rs"]
mod a1b_an_ambiguous_optional_edge_was_an_error;
#[path = "regressions/a1b_app_file_error_repeated_its_cause.rs"]
mod a1b_app_file_error_repeated_its_cause;
#[path = "regressions/a1b_app_names_were_used_as_paths.rs"]
mod a1b_app_names_were_used_as_paths;
#[path = "regressions/a1b_shadowed_otp_ambiguity_aborted_the_closure.rs"]
mod a1b_shadowed_otp_ambiguity_aborted_the_closure;
