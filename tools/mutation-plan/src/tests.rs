// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Project(PathBuf);
impl Project {
    fn new(source: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ginary-mutation-plan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("owned directory");
        std::fs::create_dir(path.join("src")).expect("source directory");
        std::fs::write(path.join("src/lib.rs"), source).expect("fixture source");
        Self(path)
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn candidate(name: &str, start: (usize, usize), end: (usize, usize)) -> Value {
    json!({"name":name,"file":"src/lib.rs","genre":"BinaryOperator", "replacement":"-",
        "span":{"start":{"line":start.0,"column":start.1},"end":{"line":end.0,"column":end.1}},
        "package":"example","diff":"authoritative diff retained","custom":{"keep":true}})
}

#[test]
fn nested_module_impl_and_function_cfg_assign_the_native_platform_without_losing_fields() {
    let project = Project::new(
        "#[cfg(unix)]\nmod native {\n #[cfg(not(target_os = \"linux\"))]\n impl Thing {\n  #[cfg(feature = \"cli\")]\n  fn answer() -> u8 { 1 + 2 }\n }\n}\n",
    );
    let original = candidate("native answer", (6, 25), (6, 26));
    let report = enrich(&project.0, json!([original.clone()])).expect("an applicable candidate");
    assert_eq!(report[0]["applicability"]["platforms"], json!(["macos"]));
    assert_eq!(report[0]["applicability"]["assigned_platform"], "macos");
    assert_eq!(report[0]["custom"], original["custom"]);
    assert_eq!(report[0]["diff"], original["diff"]);
    assert_eq!(
        report[0]["applicability"]["cfg"].as_array().unwrap().len(),
        3
    );
}

#[test]
fn unknown_cfg_cannot_be_silently_classified_as_inactive() {
    let project = Project::new("#[cfg(unknown_platform)]\nfn answer() -> u8 { 1 + 2 }\n");
    let error = enrich(&project.0, json!([candidate("unknown", (2, 23), (2, 24))])).unwrap_err();
    assert!(error.contains("unknown cfg"), "{error}");
}

#[test]
fn duplicate_mutant_names_are_rejected_before_assignment() {
    let project = Project::new("fn answer() -> u8 { 1 + 2 }\n");
    let item = candidate("duplicate", (1, 23), (1, 24));
    let error = enrich(&project.0, json!([item.clone(), item])).unwrap_err();
    assert!(error.contains("duplicate"), "{error}");
}

fn token_candidate(source: &str, token: &str, name: &str) -> Value {
    let offset = source.rfind(token).expect("unique target token");
    let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
    let prefix = source[..offset].rsplit('\n').next().unwrap();
    let column = prefix.chars().count() + 1;
    candidate(name, (line, column), (line, column + token.chars().count()))
}

#[test]
fn fn_value_uses_the_function_cfg_and_not_its_first_conditional_statement() {
    let source = "fn answer() {\n #[cfg(windows)]\n { let _ = 1 + 2; }\n}\n";
    let project = Project::new(source);
    let binary = token_candidate(source, "+", "conditional operation");
    let mut whole = binary.clone();
    let syntax = syn::parse_file(source).unwrap();
    let syn::Item::Fn(function) = &syntax.items[0] else {
        panic!("fixture function")
    };
    whole["name"] = json!("whole function");
    whole["genre"] = json!("FnValue");
    whole["function"] = json!({"function_name":"answer","span":Range::of(function.span()).value()});
    let report = enrich(&project.0, json!([whole, binary])).unwrap();
    assert_eq!(
        report[0]["applicability"]["platforms"],
        json!(["linux", "windows", "macos"])
    );
    assert_eq!(report[0]["applicability"]["cfg"], json!([]));
    assert_eq!(report[1]["applicability"]["platforms"], json!(["windows"]));
}

#[test]
fn braces_and_attributes_inside_comments_and_raw_strings_are_inert() {
    let source = "fn answer() -> u8 {\n let _ = r###\" } #[cfg(windows)] mod fake { + \"###;\n /* } #[cfg(unknown)] fn fake() { /* nested */ */\n // #[cfg(not(unix))] {\n 1 + 2\n}\n";
    let project = Project::new(source);
    let report = enrich(
        &project.0,
        json!([token_candidate(source, "+", "real operation")]),
    )
    .unwrap();
    assert_eq!(
        report[0]["applicability"]["platforms"],
        json!(["linux", "windows", "macos"])
    );
    assert_eq!(report[0]["applicability"]["cfg"], json!([]));
}

#[test]
fn unicode_columns_and_crlf_lines_match_cargo_mutants_character_coordinates() {
    let source = "#[cfg(windows)]\r\nfn answer() -> u8 { let _ = \"😀漢字\"; 1 + 2 }\r\n";
    let project = Project::new(source);
    let item = token_candidate(source, "+", "Unicode operation");
    let report = enrich(&project.0, json!([item.clone()])).unwrap();
    assert_eq!(report[0]["span"], item["span"]);
    assert_eq!(report[0]["applicability"]["platforms"], json!(["windows"]));
    let mut wrong = item;
    let byte_column = source.lines().nth(1).unwrap().find('+').unwrap() + 1;
    wrong["span"]["start"]["column"] = json!(byte_column);
    wrong["span"]["end"]["column"] = json!(byte_column + 1);
    assert!(
        enrich(&project.0, json!([wrong])).is_err(),
        "byte coordinates cannot be accepted as character coordinates"
    );
}

#[test]
fn external_modules_inherit_file_inline_module_and_declaration_conditions() {
    let project =
        Project::new("#![cfg(feature = \"cli\")]\nmod outer {\n #[cfg(windows)]\n mod child;\n}\n");
    std::fs::create_dir(project.0.join("src/outer")).unwrap();
    let source = "#![cfg(feature = \"fault-injection\")]\nfn answer() -> u8 { 1 + 2 }\n";
    std::fs::write(project.0.join("src/outer/child.rs"), source).unwrap();
    let mut item = token_candidate(source, "+", "external module");
    item["file"] = json!("src/outer/child.rs");
    let report = enrich(&project.0, json!([item])).unwrap();
    assert_eq!(report[0]["applicability"]["platforms"], json!(["windows"]));
    assert_eq!(
        report[0]["applicability"]["cfg"].as_array().unwrap().len(),
        3
    );
    assert_eq!(report[0]["applicability"]["cfg"][0]["file"], "src/lib.rs");
    assert_eq!(
        report[0]["applicability"]["cfg"][2]["file"],
        "src/outer/child.rs"
    );
}

#[test]
fn all_any_not_and_feature_conditions_are_evaluated_for_every_native_platform() {
    let source = "#[cfg(all(any(windows, target_os = \"macos\"), not(test), feature = \"fault-injection\"))]\nfn answer() -> u8 { 1 + 2 }\n";
    let project = Project::new(source);
    let report = enrich(
        &project.0,
        json!([token_candidate(source, "+", "combined cfg")]),
    )
    .unwrap();
    assert_eq!(
        report[0]["applicability"]["platforms"],
        json!(["windows", "macos"])
    );
    assert_eq!(report[0]["applicability"]["assigned_platform"], "windows");
    assert_eq!(
        report[0]["applicability"]["cfg"][0]["predicate"],
        "all(any(windows, target_os = \"macos\"), not(test), feature = \"fault-injection\")"
    );
}

#[test]
fn contradictory_and_test_only_candidates_are_errors_not_dropped_rows() {
    for condition in [
        "all(unix,windows)",
        "test",
        "any()",
        "all(not(unix),not(windows))",
    ] {
        let source = format!("#[cfg({condition})]\nfn answer() -> u8 {{ 1 + 2 }}\n");
        let project = Project::new(&source);
        let error = enrich(
            &project.0,
            json!([token_candidate(&source, "+", condition)]),
        )
        .unwrap_err();
        assert!(error.contains("no applicable native platform"), "{error}");
    }
}

#[test]
fn unknown_predicates_and_malformed_not_cannot_short_circuit_out_of_review() {
    for condition in [
        "all(any(),unknown)",
        "any(all(),unknown)",
        "feature = \"unplanned\"",
        "target_arch = \"x86_64\"",
        "not(unix,windows)",
    ] {
        let source = format!("#[cfg({condition})]\nfn answer() -> u8 {{ 1 + 2 }}\n");
        let project = Project::new(&source);
        let error = enrich(
            &project.0,
            json!([token_candidate(&source, "+", condition)]),
        )
        .unwrap_err();
        assert!(error.contains("unknown cfg"), "{error}");
    }
}

#[test]
fn cfg_attr_conditions_apply_only_when_their_predicate_holds() {
    let source = "#[cfg_attr(unix, cfg(target_os = \"macos\"))]\nfn answer() -> u8 { 1 + 2 }\n";
    let project = Project::new(source);
    let report = enrich(
        &project.0,
        json!([token_candidate(source, "+", "cfg_attr")]),
    )
    .unwrap();
    assert_eq!(
        report[0]["applicability"]["platforms"],
        json!(["windows", "macos"])
    );
    let source = "#[cfg_attr(unknown, cfg(windows))]\nfn answer() -> u8 { 1 + 2 }\n";
    let project = Project::new(source);
    assert!(
        enrich(
            &project.0,
            json!([token_candidate(source, "+", "unknown cfg_attr")])
        )
        .unwrap_err()
        .contains("unknown cfg")
    );
}

#[test]
fn ambiguous_external_module_ownership_is_rejected() {
    let project =
        Project::new("#[path = \"shared.rs\"] mod first;\n#[path = \"shared.rs\"] mod second;\n");
    let source = "fn answer() -> u8 { 1 + 2 }\n";
    std::fs::write(project.0.join("src/shared.rs"), source).unwrap();
    let mut item = token_candidate(source, "+", "shared module");
    item["file"] = json!("src/shared.rs");
    assert!(
        enrich(&project.0, json!([item]))
            .unwrap_err()
            .contains("ambiguous module ownership")
    );
}

#[test]
fn explicit_external_paths_are_relative_to_the_containing_source_file() {
    let project = Project::new("mod outer;\n");
    std::fs::write(
        project.0.join("src/outer.rs"),
        "#[path = \"shared.rs\"] mod child;\n",
    )
    .unwrap();
    let source = "#[cfg(windows)]\nfn answer() -> u8 { 1 + 2 }\n";
    std::fs::write(project.0.join("src/shared.rs"), source).unwrap();
    let mut item = token_candidate(source, "+", "explicit external path");
    item["file"] = json!("src/shared.rs");
    let report = enrich(&project.0, json!([item])).unwrap();
    assert_eq!(report[0]["applicability"]["platforms"], json!(["windows"]));
}

#[test]
fn inline_module_path_attributes_cannot_select_a_different_unconditional_file() {
    let project = Project::new("#[path = \"selected\"] mod outer { mod child; }\n");
    for directory in ["src/selected", "src/outer"] {
        std::fs::create_dir(project.0.join(directory)).unwrap();
    }
    let real = "#[cfg(windows)]\nfn answer() -> u8 { 1 + 2 }\n";
    let decoy = "fn answer() -> u8 { 1 + 2 }\n";
    std::fs::write(project.0.join("src/selected/child.rs"), real).unwrap();
    std::fs::write(project.0.join("src/outer/child.rs"), decoy).unwrap();
    let mut item = token_candidate(decoy, "+", "wrong inline path must fail closed");
    item["file"] = json!("src/outer/child.rs");
    let error = enrich(&project.0, json!([item])).unwrap_err();
    assert!(error.contains("unsupported inline module path"), "{error}");
}

#[test]
fn forged_operator_locations_inside_strings_are_not_syntax_ownership() {
    let source = "fn answer() { let _ = \"+\"; }\n";
    let project = Project::new(source);
    let error = enrich(
        &project.0,
        json!([token_candidate(source, "+", "string operation")]),
    )
    .unwrap_err();
    assert!(error.contains("matching Rust syntax node"), "{error}");
}

#[test]
fn a_function_owner_from_another_function_is_rejected() {
    let source = "fn first() -> u8 { 1 + 2 }\nfn second() -> u8 { 3 + 4 }\n";
    let project = Project::new(source);
    let syntax = syn::parse_file(source).unwrap();
    let mut item = token_candidate(source, "+", "wrong owner");
    item["function"] =
        json!({"function_name":"first","span":Range::of(syntax.items[0].span()).value()});
    assert!(
        enrich(&project.0, json!([item]))
            .unwrap_err()
            .contains("outside the declared function body")
    );
}

#[test]
fn candidate_paths_and_unknown_mutation_genres_fail_closed() {
    let source = "fn answer() -> u8 { 1 + 2 }\n";
    let project = Project::new(source);
    for path in ["../outside.rs", "/absolute.rs", "src/../src/lib.rs"] {
        let mut item = token_candidate(source, "+", path);
        item["file"] = json!(path);
        assert!(
            enrich(&project.0, json!([item]))
                .unwrap_err()
                .contains("normalized relative path")
        );
    }
    let mut item = token_candidate(source, "+", "new mutation kind");
    item["genre"] = json!("FutureMutant");
    assert!(
        enrich(&project.0, json!([item]))
            .unwrap_err()
            .contains("unsupported mutation genre")
    );
}

#[test]
fn cfg_on_variants_fields_and_generic_parameters_owns_their_nested_expressions() {
    for source in [
        "enum Example { #[cfg(windows)] Value = 1 + 2 }\n",
        "struct Example { #[cfg(windows)] value: [u8; 1 + 2] }\n",
        "struct Example<#[cfg(windows)] const N: usize = { 1 + 2 }> {}\n",
        "type Example = fn(#[cfg(windows)] [u8; 1 + 2]);\n",
    ] {
        let project = Project::new(source);
        let report = enrich(
            &project.0,
            json!([token_candidate(source, "+", "conditional member")]),
        )
        .unwrap();
        assert_eq!(
            report[0]["applicability"]["platforms"],
            json!(["windows"]),
            "{source}"
        );
        assert_eq!(
            report[0]["applicability"]["cfg"].as_array().unwrap().len(),
            1,
            "attribute evidence is recorded once"
        );
    }
}
