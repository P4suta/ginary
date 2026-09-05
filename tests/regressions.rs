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

#[path = "regressions/e8_a_listing_path_was_spelled_the_way_the_platform_does.rs"]
mod e8_a_listing_path_was_spelled_the_way_the_platform_does;
#[path = "regressions/e8_a_removal_renamed_a_directory_it_still_held_open.rs"]
mod e8_a_removal_renamed_a_directory_it_still_held_open;
#[cfg(feature = "cli")]
#[path = "regressions/e8_a_windows_build_could_not_locate_its_cache_directory.rs"]
mod e8_a_windows_build_could_not_locate_its_cache_directory;
#[cfg(feature = "cli")]
#[path = "regressions/e8_a_windows_host_could_not_bundle_its_own_runtime.rs"]
mod e8_a_windows_host_could_not_bundle_its_own_runtime;
#[path = "regressions/e8_the_ad_hoc_signature_did_not_cover_the_finished_file.rs"]
mod e8_the_ad_hoc_signature_did_not_cover_the_finished_file;
#[path = "regressions/e8_the_beam_step_looked_for_a_program_windows_spells_otherwise.rs"]
mod e8_the_beam_step_looked_for_a_program_windows_spells_otherwise;
#[cfg(feature = "cli")]
#[path = "regressions/e8_the_cache_probe_wrote_a_program_the_platform_cannot_start.rs"]
mod e8_the_cache_probe_wrote_a_program_the_platform_cannot_start;
#[path = "regressions/e8_the_extraction_flushed_handles_it_had_opened_read_only.rs"]
mod e8_the_extraction_flushed_handles_it_had_opened_read_only;
#[cfg(feature = "cli")]
#[path = "regressions/e8_the_injected_segment_broke_page_alignment.rs"]
mod e8_the_injected_segment_broke_page_alignment;
#[path = "regressions/e8_two_producers_disagreed_about_one_files_mode.rs"]
mod e8_two_producers_disagreed_about_one_files_mode;

#[path = "regressions/e10_a_fake_otp_wrote_an_erl_windows_cannot_start.rs"]
mod e10_a_fake_otp_wrote_an_erl_windows_cannot_start;
#[path = "regressions/e10_a_snapshot_pinned_the_hosts_own_path_spelling.rs"]
mod e10_a_snapshot_pinned_the_hosts_own_path_spelling;
#[path = "regressions/e10_a_test_asked_posix_whether_a_windows_path_was_absolute.rs"]
mod e10_a_test_asked_posix_whether_a_windows_path_was_absolute;
#[cfg(feature = "cli")]
#[path = "regressions/e10_an_x86_64_stub_had_no_code_signature_to_reuse.rs"]
mod e10_an_x86_64_stub_had_no_code_signature_to_reuse;
#[path = "regressions/e10_the_needle_halves_were_stored_side_by_side.rs"]
mod e10_the_needle_halves_were_stored_side_by_side;

#[cfg(feature = "cli")]
#[path = "regressions/e11_a_dll_the_import_table_spelt_in_lower_case_was_unexpected.rs"]
mod e11_a_dll_the_import_table_spelt_in_lower_case_was_unexpected;
#[path = "regressions/e11_a_fixture_built_a_directory_windows_cannot_name.rs"]
mod e11_a_fixture_built_a_directory_windows_cannot_name;
#[path = "regressions/e11_a_listing_path_was_joined_the_way_the_host_spells_one.rs"]
mod e11_a_listing_path_was_joined_the_way_the_host_spells_one;
#[path = "regressions/e11_a_live_process_fixture_needed_bin_sh.rs"]
mod e11_a_live_process_fixture_needed_bin_sh;
#[path = "regressions/e11_a_path_in_a_json_trace_was_looked_for_unescaped.rs"]
mod e11_a_path_in_a_json_trace_was_looked_for_unescaped;
#[cfg(feature = "cli")]
#[path = "regressions/e11_a_pe_sentence_carried_two_raw_nul_bytes.rs"]
mod e11_a_pe_sentence_carried_two_raw_nul_bytes;
#[cfg(feature = "cli")]
#[path = "regressions/e11_a_shell_script_test_ran_on_a_host_with_no_posix_shell.rs"]
mod e11_a_shell_script_test_ran_on_a_host_with_no_posix_shell;
#[cfg(feature = "cli")]
#[path = "regressions/e11_a_stub_search_target_was_the_host_on_one_machine.rs"]
mod e11_a_stub_search_target_was_the_host_on_one_machine;
#[cfg(feature = "cli")]
#[path = "regressions/e11_a_tree_of_objects_the_stripper_cannot_read_was_silent.rs"]
mod e11_a_tree_of_objects_the_stripper_cannot_read_was_silent;
#[cfg(feature = "cli")]
#[path = "regressions/e11_an_artifact_run_isolated_its_cache_on_one_platform_only.rs"]
mod e11_an_artifact_run_isolated_its_cache_on_one_platform_only;
#[cfg(feature = "cli")]
#[path = "regressions/e11_an_import_table_that_would_not_parse_read_as_no_imports.rs"]
mod e11_an_import_table_that_would_not_parse_read_as_no_imports;
#[cfg(feature = "cli")]
#[path = "regressions/e11_doctor_looked_for_a_crypto_nif_windows_spells_otherwise.rs"]
mod e11_doctor_looked_for_a_crypto_nif_windows_spells_otherwise;
#[path = "regressions/e11_extract_only_printed_the_spelling_only_ginary_opens.rs"]
mod e11_extract_only_printed_the_spelling_only_ginary_opens;
#[cfg(feature = "cli")]
#[path = "regressions/e11_the_argv_log_was_read_under_the_programs_unix_name.rs"]
mod e11_the_argv_log_was_read_under_the_programs_unix_name;
#[cfg(feature = "cli")]
#[path = "regressions/e11_the_beam_argv_named_the_unix_bit_bucket.rs"]
mod e11_the_beam_argv_named_the_unix_bit_bucket;
#[cfg(feature = "cli")]
#[path = "regressions/e11_the_deep_check_read_only_one_of_the_three_object_formats.rs"]
mod e11_the_deep_check_read_only_one_of_the_three_object_formats;
#[path = "regressions/e11_the_emulator_was_looked_for_under_its_unix_name.rs"]
mod e11_the_emulator_was_looked_for_under_its_unix_name;
#[path = "regressions/e11_the_fixture_server_tore_down_a_connection_it_had_just_answered.rs"]
mod e11_the_fixture_server_tore_down_a_connection_it_had_just_answered;
#[cfg(feature = "cli")]
#[path = "regressions/e11_the_running_executable_was_taken_for_an_elf.rs"]
mod e11_the_running_executable_was_taken_for_an_elf;
#[cfg(feature = "cli")]
#[path = "regressions/e11_the_temporary_fallback_was_named_after_one_platforms_variable.rs"]
mod e11_the_temporary_fallback_was_named_after_one_platforms_variable;
#[cfg(feature = "cli")]
#[path = "regressions/e12_a_crypto_fixture_planted_the_unix_nif_for_a_host_probe.rs"]
mod e12_a_crypto_fixture_planted_the_unix_nif_for_a_host_probe;
#[cfg(feature = "cli")]
#[path = "regressions/e12_a_nested_json_trace_path_was_escaped_once_and_written_twice.rs"]
mod e12_a_nested_json_trace_path_was_escaped_once_and_written_twice;
#[path = "regressions/e12_a_printed_working_directory_was_compared_as_text.rs"]
mod e12_a_printed_working_directory_was_compared_as_text;
#[cfg(feature = "cli")]
#[path = "regressions/e12_a_required_erts_name_was_reported_missing_from_the_tree_holding_it.rs"]
mod e12_a_required_erts_name_was_reported_missing_from_the_tree_holding_it;
#[cfg(feature = "cli")]
#[path = "regressions/e12_a_windows_artifact_carried_the_debug_emulator.rs"]
mod e12_a_windows_artifact_carried_the_debug_emulator;
#[cfg(feature = "cli")]
#[path = "regressions/e12_the_cross_target_a_stub_test_used_had_no_name.rs"]
mod e12_the_cross_target_a_stub_test_used_had_no_name;
#[cfg(feature = "cli")]
#[path = "regressions/e12_the_real_artifact_check_named_the_unix_emulator.rs"]
mod e12_the_real_artifact_check_named_the_unix_emulator;
#[cfg(feature = "cli")]
#[path = "regressions/e12_the_sweep_asked_proc_whether_a_process_was_alive.rs"]
mod e12_the_sweep_asked_proc_whether_a_process_was_alive;
#[cfg(feature = "cli")]
#[path = "regressions/e12_the_windows_allowlist_carried_one_vc_runtime_of_three.rs"]
mod e12_the_windows_allowlist_carried_one_vc_runtime_of_three;
#[path = "regressions/e12_three_statements_of_the_unsafe_exception_said_three_calls.rs"]
mod e12_three_statements_of_the_unsafe_exception_said_three_calls;
#[cfg(feature = "cli")]
#[path = "regressions/e13_a_compressed_document_was_bounded_by_its_wire_bytes.rs"]
mod e13_a_compressed_document_was_bounded_by_its_wire_bytes;
#[cfg(feature = "cli")]
#[path = "regressions/e13_a_document_over_the_bound_was_asked_for_twice_more.rs"]
mod e13_a_document_over_the_bound_was_asked_for_twice_more;
#[cfg(feature = "cli")]
#[path = "regressions/e13_a_document_that_is_not_text_was_asked_for_twice_more.rs"]
mod e13_a_document_that_is_not_text_was_asked_for_twice_more;
#[path = "regressions/e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs"]
mod e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence;
#[path = "regressions/e13_a_request_the_fixture_could_not_read_was_dropped_in_silence.rs"]
mod e13_a_request_the_fixture_could_not_read_was_dropped_in_silence;
#[path = "regressions/e13_the_fixture_server_inherited_its_listeners_non_blocking_mode.rs"]
mod e13_the_fixture_server_inherited_its_listeners_non_blocking_mode;
#[path = "regressions/e13_the_fixture_server_stopped_on_an_error_a_peer_can_cause.rs"]
mod e13_the_fixture_server_stopped_on_an_error_a_peer_can_cause;
#[path = "regressions/e15_a_pwsh_step_ended_with_the_code_it_asserted.rs"]
mod e15_a_pwsh_step_ended_with_the_code_it_asserted;
#[path = "regressions/e15_the_adr_credited_the_windows_job_with_a_spawn_that_never_ran.rs"]
mod e15_the_adr_credited_the_windows_job_with_a_spawn_that_never_ran;
#[cfg(feature = "cli")]
#[path = "regressions/e16_a_cached_macos_runtime_was_read_by_the_elf_reader.rs"]
mod e16_a_cached_macos_runtime_was_read_by_the_elf_reader;
#[cfg(feature = "cli")]
#[path = "regressions/e16_a_cached_windows_runtime_was_read_by_the_elf_reader.rs"]
mod e16_a_cached_windows_runtime_was_read_by_the_elf_reader;
#[path = "regressions/e16_a_glibc_only_assertion_ran_under_a_linux_gate.rs"]
mod e16_a_glibc_only_assertion_ran_under_a_linux_gate;
#[path = "regressions/e16_a_glibc_only_expectation_was_asserted_on_any_elf_host.rs"]
mod e16_a_glibc_only_expectation_was_asserted_on_any_elf_host;
#[path = "regressions/e17_a_step_that_never_runs_read_as_one_that_always_does.rs"]
mod e17_a_step_that_never_runs_read_as_one_that_always_does;
#[path = "regressions/e17_the_notice_named_the_environment_by_accident.rs"]
mod e17_the_notice_named_the_environment_by_accident;
#[path = "regressions/e17_the_release_credentials_were_read_outside_their_environment.rs"]
mod e17_the_release_credentials_were_read_outside_their_environment;

#[path = "regressions/e18_a_credential_reading_step_outside_the_check_job_was_not_scanned.rs"]
mod e18_a_credential_reading_step_outside_the_check_job_was_not_scanned;
#[path = "regressions/e18_a_failing_step_was_taken_for_the_notice.rs"]
mod e18_a_failing_step_was_taken_for_the_notice;
#[path = "regressions/e18_a_step_that_was_not_the_notice_wore_the_notice_guard.rs"]
mod e18_a_step_that_was_not_the_notice_wore_the_notice_guard;
#[path = "regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs"]
mod e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials;
#[path = "regressions/e18_the_branch_policy_was_claimed_without_the_bypass_beside_it.rs"]
mod e18_the_branch_policy_was_claimed_without_the_bypass_beside_it;
#[path = "regressions/e18_the_environment_was_credited_with_keeping_other_jobs_out.rs"]
mod e18_the_environment_was_credited_with_keeping_other_jobs_out;
#[path = "regressions/e18_the_notice_overclaimed_what_repository_scope_exposes.rs"]
mod e18_the_notice_overclaimed_what_repository_scope_exposes;

#[path = "regressions/e19_a_repository_property_test_could_not_answer_in_a_copy_of_the_tree.rs"]
mod e19_a_repository_property_test_could_not_answer_in_a_copy_of_the_tree;
#[path = "regressions/e19_the_fuzz_smoke_built_for_the_triple_cargo_fuzz_was_installed_for.rs"]
mod e19_the_fuzz_smoke_built_for_the_triple_cargo_fuzz_was_installed_for;

#[path = "regressions/e20_a_compact_manifest_read_as_one_with_no_entry.rs"]
mod e20_a_compact_manifest_read_as_one_with_no_entry;
#[path = "regressions/e20_a_dangling_changelog_link_was_pinned_by_its_tag.rs"]
mod e20_a_dangling_changelog_link_was_pinned_by_its_tag;
#[path = "regressions/e20_a_missing_cargo_toml_reported_a_libc_error.rs"]
mod e20_a_missing_cargo_toml_reported_a_libc_error;
#[path = "regressions/e20_a_removal_the_cleaner_rule_could_not_see.rs"]
mod e20_a_removal_the_cleaner_rule_could_not_see;
#[path = "regressions/e20_a_workflow_could_point_the_version_check_at_another_tree.rs"]
mod e20_a_workflow_could_point_the_version_check_at_another_tree;
#[path = "regressions/e20_release_please_did_not_rewrite_the_unreleased_section.rs"]
mod e20_release_please_did_not_rewrite_the_unreleased_section;
#[path = "regressions/e20_the_cleaner_deleted_the_directory_it_was_run_from.rs"]
mod e20_the_cleaner_deleted_the_directory_it_was_run_from;
