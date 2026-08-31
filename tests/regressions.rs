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
#[path = "regressions/a4_a_non_utf8_output_path_failed_the_json_report.rs"]
mod a4_a_non_utf8_output_path_failed_the_json_report;
#[path = "regressions/a4_a_work_directory_that_could_not_be_removed_was_unreported.rs"]
mod a4_a_work_directory_that_could_not_be_removed_was_unreported;
#[path = "regressions/a4_an_unreadable_trailer_was_a_damaged_artifact.rs"]
mod a4_an_unreadable_trailer_was_a_damaged_artifact;
#[path = "regressions/a4_extra_bin_names_were_used_as_paths.rs"]
mod a4_extra_bin_names_were_used_as_paths;
#[path = "regressions/b1_a_locked_entry_blocked_the_launch.rs"]
mod b1_a_locked_entry_blocked_the_launch;
#[path = "regressions/b1_a_prune_that_could_not_rename_reported_nothing.rs"]
mod b1_a_prune_that_could_not_rename_reported_nothing;
#[path = "regressions/b1_an_unterminated_quote_hid_the_rest_of_an_args_file.rs"]
mod b1_an_unterminated_quote_hid_the_rest_of_an_args_file;
#[path = "regressions/b1_heart_command_was_not_shell_quoted.rs"]
mod b1_heart_command_was_not_shell_quoted;
#[path = "regressions/b1_manifest_env_overrode_the_launcher_s_own.rs"]
mod b1_manifest_env_overrode_the_launcher_s_own;
#[path = "regressions/b1_the_entry_could_vanish_between_the_preflight_and_the_lock.rs"]
mod b1_the_entry_could_vanish_between_the_preflight_and_the_lock;
#[path = "regressions/b1_the_prune_trace_named_nothing_it_removed.rs"]
mod b1_the_prune_trace_named_nothing_it_removed;
#[path = "regressions/b1_uninstall_removed_the_crash_dump.rs"]
mod b1_uninstall_removed_the_crash_dump;
#[path = "regressions/b2_a_directory_entry_was_an_index_orphan.rs"]
mod b2_a_directory_entry_was_an_index_orphan;
#[path = "regressions/b2_a_file_that_was_not_a_dump_echoed_its_bytes.rs"]
mod b2_a_file_that_was_not_a_dump_echoed_its_bytes;
#[path = "regressions/b2_a_reserved_name_past_the_front_matter_was_skipped.rs"]
mod b2_a_reserved_name_past_the_front_matter_was_skipped;
#[path = "regressions/b2_a_section_on_the_second_line_was_dropped.rs"]
mod b2_a_section_on_the_second_line_was_dropped;
#[path = "regressions/b2_an_escaping_entry_verified_clean.rs"]
mod b2_an_escaping_entry_verified_clean;
#[path = "regressions/b2_build_sbom_out_hid_the_artifact_it_had_written.rs"]
mod b2_build_sbom_out_hid_the_artifact_it_had_written;
