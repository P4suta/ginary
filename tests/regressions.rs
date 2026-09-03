// SPDX-License-Identifier: MIT OR Apache-2.0
//! The regression suite: one module per fixed bug.
//!
//! Cargo compiles only the `.rs` files directly under `tests/` as test targets,
//! so the files in `tests/regressions/` are included from here. See
//! `tests/regressions/README.md` for the convention each of them follows.

mod common;

#[cfg(feature = "cli")]
#[path = "regressions/a1a_display_left_reserved_words_bare.rs"]
mod a1a_display_left_reserved_words_bare;
#[cfg(feature = "cli")]
#[path = "regressions/a1a_doctor_dropped_the_otp_error.rs"]
mod a1a_doctor_dropped_the_otp_error;
#[cfg(feature = "cli")]
#[path = "regressions/a1a_env_duplicate_keys_were_unreported.rs"]
mod a1a_env_duplicate_keys_were_unreported;
#[cfg(feature = "cli")]
#[path = "regressions/a1b_an_ambiguous_optional_edge_was_an_error.rs"]
mod a1b_an_ambiguous_optional_edge_was_an_error;
#[cfg(feature = "cli")]
#[path = "regressions/a1b_app_file_error_repeated_its_cause.rs"]
mod a1b_app_file_error_repeated_its_cause;
#[cfg(feature = "cli")]
#[path = "regressions/a1b_app_names_were_used_as_paths.rs"]
mod a1b_app_names_were_used_as_paths;
#[cfg(feature = "cli")]
#[path = "regressions/a1b_shadowed_otp_ambiguity_aborted_the_closure.rs"]
mod a1b_shadowed_otp_ambiguity_aborted_the_closure;
#[cfg(feature = "cli")]
#[path = "regressions/a1c_a_non_utf8_file_name_was_dropped.rs"]
mod a1c_a_non_utf8_file_name_was_dropped;
#[cfg(feature = "cli")]
#[path = "regressions/a1c_a_symlinked_directory_looped_or_leaked.rs"]
mod a1c_a_symlinked_directory_looped_or_leaked;
#[cfg(feature = "cli")]
#[path = "regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs"]
mod a1c_a_symlinked_ebin_or_priv_escaped_the_app;
#[cfg(feature = "cli")]
#[path = "regressions/a2_a_module_outside_ebin_was_never_stripped.rs"]
mod a2_a_module_outside_ebin_was_never_stripped;
#[cfg(feature = "cli")]
#[path = "regressions/a2_a_shared_object_with_an_interpreter_was_fully_stripped.rs"]
mod a2_a_shared_object_with_an_interpreter_was_fully_stripped;
#[cfg(feature = "cli")]
#[path = "regressions/a2_a_symlinked_priv_reached_an_excluded_directory.rs"]
mod a2_a_symlinked_priv_reached_an_excluded_directory;
#[cfg(feature = "cli")]
#[path = "regressions/a2_an_unreadable_elf_file_failed_the_whole_stage.rs"]
mod a2_an_unreadable_elf_file_failed_the_whole_stage;
#[cfg(feature = "cli")]
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
#[cfg(feature = "cli")]
#[path = "regressions/a3b_cache_clean_app_escaped_the_root.rs"]
mod a3b_cache_clean_app_escaped_the_root;
#[path = "regressions/a3b_the_manifest_app_was_not_a_name.rs"]
mod a3b_the_manifest_app_was_not_a_name;
#[path = "regressions/a3b_the_move_aside_branch_deleted_a_complete_entry.rs"]
mod a3b_the_move_aside_branch_deleted_a_complete_entry;
#[cfg(feature = "cli")]
#[path = "regressions/a4_a_non_utf8_output_path_failed_the_json_report.rs"]
mod a4_a_non_utf8_output_path_failed_the_json_report;
#[cfg(feature = "cli")]
#[path = "regressions/a4_a_work_directory_that_could_not_be_removed_was_unreported.rs"]
mod a4_a_work_directory_that_could_not_be_removed_was_unreported;
#[cfg(feature = "cli")]
#[path = "regressions/a4_an_unreadable_trailer_was_a_damaged_artifact.rs"]
mod a4_an_unreadable_trailer_was_a_damaged_artifact;
#[cfg(feature = "cli")]
#[path = "regressions/a4_extra_bin_names_were_used_as_paths.rs"]
mod a4_extra_bin_names_were_used_as_paths;
#[path = "regressions/b1_a_locked_entry_blocked_the_launch.rs"]
mod b1_a_locked_entry_blocked_the_launch;
#[path = "regressions/b1_a_prune_that_could_not_rename_reported_nothing.rs"]
mod b1_a_prune_that_could_not_rename_reported_nothing;
#[cfg(feature = "cli")]
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
#[cfg(feature = "cli")]
#[path = "regressions/b2_a_directory_entry_was_an_index_orphan.rs"]
mod b2_a_directory_entry_was_an_index_orphan;
#[cfg(feature = "cli")]
#[path = "regressions/b2_a_file_that_was_not_a_dump_echoed_its_bytes.rs"]
mod b2_a_file_that_was_not_a_dump_echoed_its_bytes;
#[cfg(feature = "cli")]
#[path = "regressions/b2_a_reserved_name_past_the_front_matter_was_skipped.rs"]
mod b2_a_reserved_name_past_the_front_matter_was_skipped;
#[cfg(feature = "cli")]
#[path = "regressions/b2_a_section_on_the_second_line_was_dropped.rs"]
mod b2_a_section_on_the_second_line_was_dropped;
#[cfg(feature = "cli")]
#[path = "regressions/b2_an_escaping_entry_verified_clean.rs"]
mod b2_an_escaping_entry_verified_clean;
#[cfg(feature = "cli")]
#[path = "regressions/b2_build_sbom_out_hid_the_artifact_it_had_written.rs"]
mod b2_build_sbom_out_hid_the_artifact_it_had_written;
#[cfg(feature = "cli")]
#[path = "regressions/b2_verify_compared_only_the_digest.rs"]
mod b2_verify_compared_only_the_digest;
#[cfg(feature = "cli")]
#[path = "regressions/c1_a_non_utf8_output_path_lost_its_suffix.rs"]
mod c1_a_non_utf8_output_path_lost_its_suffix;
#[cfg(feature = "cli")]
#[path = "regressions/c1_a_runtime_that_could_not_be_read_still_resolved.rs"]
mod c1_a_runtime_that_could_not_be_read_still_resolved;
#[cfg(feature = "cli")]
#[path = "regressions/c2_a_special_file_stub_said_not_a_regular_file_rather_than_a_file.rs"]
mod c2_a_special_file_stub_said_not_a_regular_file_rather_than_a_file;
#[cfg(feature = "cli")]
#[path = "regressions/c2_a_stub_that_was_a_directory_was_not_there.rs"]
mod c2_a_stub_that_was_a_directory_was_not_there;
#[cfg(feature = "cli")]
#[path = "regressions/c2_a_target_sub_table_with_no_erts_passed_the_guard.rs"]
mod c2_a_target_sub_table_with_no_erts_passed_the_guard;
#[cfg(feature = "cli")]
#[path = "regressions/c2_the_artifact_never_had_to_use_the_stub.rs"]
mod c2_the_artifact_never_had_to_use_the_stub;
#[cfg(feature = "cli")]
#[path = "regressions/c2_the_pe_gate_was_never_exercised.rs"]
mod c2_the_pe_gate_was_never_exercised;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_catalog_release_warning_reached_only_the_trace.rs"]
mod c3_a_catalog_release_warning_reached_only_the_trace;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_docker_source_named_the_catalog_milestone.rs"]
mod c3_a_docker_source_named_the_catalog_milestone;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_foreign_native_left_the_strip_report_silent.rs"]
mod c3_a_foreign_native_left_the_strip_report_silent;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_hard_wrapped_message_ran_its_words_together.rs"]
mod c3_a_hard_wrapped_message_ran_its_words_together;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_repacked_runtime_carried_a_non_zero_mtime.rs"]
mod c3_a_repacked_runtime_carried_a_non_zero_mtime;
#[cfg(feature = "cli")]
#[path = "regressions/c3_a_variant_the_catalog_names_was_refused_by_the_config.rs"]
mod c3_a_variant_the_catalog_names_was_refused_by_the_config;
#[cfg(feature = "cli")]
#[path = "regressions/c3_otp_update_truncated_the_catalog_it_replaced.rs"]
mod c3_otp_update_truncated_the_catalog_it_replaced;
#[cfg(feature = "cli")]
#[path = "regressions/c3_the_not_cached_remedy_dropped_the_flags_it_was_given.rs"]
mod c3_the_not_cached_remedy_dropped_the_flags_it_was_given;
#[cfg(feature = "cli")]
#[path = "regressions/c3_the_otp_cache_was_filled_without_a_lock.rs"]
mod c3_the_otp_cache_was_filled_without_a_lock;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_catalog_path_with_a_space_was_not_quoted.rs"]
mod c4_a_catalog_path_with_a_space_was_not_quoted;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_hook_ran_before_a_refusal_it_could_not_lift.rs"]
mod c4_a_hook_ran_before_a_refusal_it_could_not_lift;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_hook_token_was_pasted_unquoted.rs"]
mod c4_a_hook_token_was_pasted_unquoted;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_position_independent_program_was_a_shared_object.rs"]
mod c4_a_position_independent_program_was_a_shared_object;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_shipment_app_outside_the_closure_stopped_the_build.rs"]
mod c4_a_shipment_app_outside_the_closure_stopped_the_build;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_shipment_doctor_could_not_scan_said_nothing.rs"]
mod c4_a_shipment_doctor_could_not_scan_said_nothing;
#[cfg(feature = "cli")]
#[path = "regressions/c4_a_static_musl_runtime_was_reported_ok.rs"]
mod c4_a_static_musl_runtime_was_reported_ok;
#[cfg(feature = "cli")]
#[path = "regressions/c4_the_hook_shell_was_cmd_on_a_windows_host.rs"]
mod c4_the_hook_shell_was_cmd_on_a_windows_host;
#[cfg(feature = "cli")]
#[path = "regressions/c4_the_kind_column_disagreed_with_the_verdict.rs"]
mod c4_the_kind_column_disagreed_with_the_verdict;
#[path = "regressions/d2_a_removal_walked_the_ordinary_spelling_of_a_verbatim_tree.rs"]
mod d2_a_removal_walked_the_ordinary_spelling_of_a_verbatim_tree;

#[path = "regressions/d2_a_windows_artifact_could_not_pass_preflight.rs"]
mod d2_a_windows_artifact_could_not_pass_preflight;
#[cfg(feature = "cli")]
#[path = "regressions/d2_a_windows_runtime_root_could_not_be_resolved.rs"]
mod d2_a_windows_runtime_root_could_not_be_resolved;
#[cfg(feature = "cli")]
#[path = "regressions/d2_a_windows_runtime_was_staged_without_its_resolver.rs"]
mod d2_a_windows_runtime_was_staged_without_its_resolver;
#[path = "regressions/d2_the_extraction_dropped_the_verbatim_prefix_after_unpacking.rs"]
mod d2_the_extraction_dropped_the_verbatim_prefix_after_unpacking;
#[cfg(feature = "cli")]
#[path = "regressions/d2_the_kind_column_called_a_stated_e_type_unknown.rs"]
mod d2_the_kind_column_called_a_stated_e_type_unknown;
#[path = "regressions/d2_the_launcher_opened_a_path_it_had_not_written.rs"]
mod d2_the_launcher_opened_a_path_it_had_not_written;
#[path = "regressions/d2_the_windows_prune_could_not_rename_what_it_locked.rs"]
mod d2_the_windows_prune_could_not_rename_what_it_locked;
#[path = "regressions/d2_the_windows_shared_lock_created_the_file_it_shared.rs"]
mod d2_the_windows_shared_lock_created_the_file_it_shared;

#[cfg(feature = "cli")]
#[path = "regressions/d3_macho_section_addr_was_never_shifted.rs"]
mod d3_macho_section_addr_was_never_shifted;
#[cfg(feature = "cli")]
#[path = "regressions/d3_macho_segment_vmaddr_and_vmsize_were_wrong.rs"]
mod d3_macho_segment_vmaddr_and_vmsize_were_wrong;

#[path = "regressions/e1_otp_tarballs_escaped_the_verification.rs"]
mod e1_otp_tarballs_escaped_the_verification;
#[path = "regressions/e1_the_sha256sums_step_read_and_wrote_one_file.rs"]
mod e1_the_sha256sums_step_read_and_wrote_one_file;

#[path = "regressions/e3_an_issue_form_was_not_valid_yaml.rs"]
mod e3_an_issue_form_was_not_valid_yaml;

#[path = "regressions/e4_a_crlf_checkout_rewrote_the_hashed_fixtures.rs"]
mod e4_a_crlf_checkout_rewrote_the_hashed_fixtures;

#[path = "regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs"]
mod e5_a_gated_test_defaulted_to_one_developers_machine;

#[path = "regressions/e5_one_cross_target_directory_was_shared_between_images.rs"]
mod e5_one_cross_target_directory_was_shared_between_images;

#[path = "regressions/e5_the_build_script_had_no_name_for_the_msvc_triple.rs"]
mod e5_the_build_script_had_no_name_for_the_msvc_triple;

#[path = "regressions/e5_the_credentials_notice_was_not_tied_to_the_missing_credentials.rs"]
mod e5_the_credentials_notice_was_not_tied_to_the_missing_credentials;

#[path = "regressions/e5_the_macos_job_ran_the_stub_as_the_command_line_tool.rs"]
mod e5_the_macos_job_ran_the_stub_as_the_command_line_tool;

#[path = "regressions/e6_five_stub_gated_tests_ran_in_no_ci_job.rs"]
mod e6_five_stub_gated_tests_ran_in_no_ci_job;

#[cfg(feature = "cli")]
#[path = "regressions/e6_the_coverage_floor_measured_a_stubless_subset.rs"]
mod e6_the_coverage_floor_measured_a_stubless_subset;

#[cfg(feature = "cli")]
#[path = "regressions/e6_the_macos_job_passed_a_flag_the_cli_does_not_have.rs"]
mod e6_the_macos_job_passed_a_flag_the_cli_does_not_have;

#[path = "regressions/e6_the_macos_matrix_asked_for_a_runner_github_retired.rs"]
mod e6_the_macos_matrix_asked_for_a_runner_github_retired;

#[path = "regressions/e6_the_test_helpers_did_not_compile_on_windows.rs"]
mod e6_the_test_helpers_did_not_compile_on_windows;

#[cfg(feature = "cli")]
#[path = "regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs"]
mod e6_the_toolchain_flag_required_a_cross_stub_nobody_built;

#[path = "regressions/e7_actionlint_was_required_of_every_toolchain_job.rs"]
mod e7_actionlint_was_required_of_every_toolchain_job;

#[cfg(feature = "cli")]
#[path = "regressions/e7_a_macos_runtime_was_read_as_an_elf.rs"]
mod e7_a_macos_runtime_was_read_as_an_elf;

#[cfg(feature = "cli")]
#[path = "regressions/e7_a_real_artifact_had_to_verify_on_the_hosts_own_erlang.rs"]
mod e7_a_real_artifact_had_to_verify_on_the_hosts_own_erlang;

#[path = "regressions/e7_the_home_directory_scan_only_worked_on_one_machine.rs"]
mod e7_the_home_directory_scan_only_worked_on_one_machine;

#[path = "regressions/e7_the_unit_tests_asked_the_host_what_platform_it_was.rs"]
mod e7_the_unit_tests_asked_the_host_what_platform_it_was;

#[cfg(feature = "cli")]
#[path = "regressions/e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path.rs"]
mod e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path;

#[path = "regressions/e7_a_cargo_test_step_could_stop_at_the_first_failing_target.rs"]
mod e7_a_cargo_test_step_could_stop_at_the_first_failing_target;
