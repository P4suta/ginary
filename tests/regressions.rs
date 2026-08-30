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
#[path = "regressions/a1c_a_non_utf8_file_name_was_dropped.rs"]
mod a1c_a_non_utf8_file_name_was_dropped;
#[path = "regressions/a1c_a_symlinked_directory_looped_or_leaked.rs"]
mod a1c_a_symlinked_directory_looped_or_leaked;
#[path = "regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs"]
mod a1c_a_symlinked_ebin_or_priv_escaped_the_app;
#[path = "regressions/a2_a_module_outside_ebin_was_never_stripped.rs"]
mod a2_a_module_outside_ebin_was_never_stripped;
#[path = "regressions/a2_a_shared_object_with_an_interpreter_was_fully_stripped.rs"]
mod a2_a_shared_object_with_an_interpreter_was_fully_stripped;
#[path = "regressions/a2_a_symlinked_priv_reached_an_excluded_directory.rs"]
mod a2_a_symlinked_priv_reached_an_excluded_directory;
#[path = "regressions/a2_an_unreadable_elf_file_failed_the_whole_stage.rs"]
mod a2_an_unreadable_elf_file_failed_the_whole_stage;
#[path = "regressions/a2_the_staged_root_became_a_wildcard.rs"]
mod a2_the_staged_root_became_a_wildcard;
#[path = "regressions/a3a_a_contiguous_entry_was_extracted.rs"]
mod a3a_a_contiguous_entry_was_extracted;
#[path = "regressions/a3a_a_rejected_payload_left_its_manifest_behind.rs"]
mod a3a_a_rejected_payload_left_its_manifest_behind;
#[path = "regressions/a3a_a_repeated_front_entry_forged_the_marker.rs"]
mod a3a_a_repeated_front_entry_forged_the_marker;
#[path = "regressions/a3a_a_zero_length_payload_looked_truncated.rs"]
mod a3a_a_zero_length_payload_looked_truncated;
#[path = "regressions/a3a_the_second_payload_entry_was_never_checked.rs"]
mod a3a_the_second_payload_entry_was_never_checked;
#[path = "regressions/a3b_a_reserved_name_covered_only_the_exact_path.rs"]
mod a3b_a_reserved_name_covered_only_the_exact_path;
#[path = "regressions/a3b_cache_clean_app_escaped_the_root.rs"]
mod a3b_cache_clean_app_escaped_the_root;
#[path = "regressions/a3b_the_manifest_app_was_not_a_name.rs"]
mod a3b_the_manifest_app_was_not_a_name;
#[path = "regressions/a3b_the_move_aside_branch_deleted_a_complete_entry.rs"]
mod a3b_the_move_aside_branch_deleted_a_complete_entry;
