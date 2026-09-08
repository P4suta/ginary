// SPDX-License-Identifier: MIT OR Apache-2.0
//! CLI for the standalone native-platform mutation planner.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let mut options = BTreeMap::new();
    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            println!(
                "ginary-mutation-plan --source-root PATH --input PLAN.json --output ENRICHED.json\n\
                Reads a cargo-mutants --list --json --diff array and records Linux/Windows/macOS applicability.\n\
                Features cli and fault-injection are enabled; test=false. Refuses existing output files."
            );
            return Ok(());
        }
        let name = argument.to_str().ok_or("option name is not UTF-8")?;
        if !["--source-root", "--input", "--output"].contains(&name) {
            return Err(format!("unknown option: {name}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{name} requires a path"))?;
        if options
            .insert(name.to_owned(), PathBuf::from(value))
            .is_some()
        {
            return Err(format!("duplicate option: {name}"));
        }
    }
    let get = |name: &str| {
        options
            .get(name)
            .ok_or_else(|| format!("missing required option: {name}"))
    };
    let root = get("--source-root")?;
    let input = get("--input")?;
    let output = get("--output")?;
    let data =
        std::fs::read(input).map_err(|e| format!("cannot read input {}: {e}", input.display()))?;
    let plan = serde_json::from_slice(data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&data))
        .map_err(|e| format!("invalid input JSON: {e}"))?;
    let enriched = ginary_mutation_plan::enrich(root, plan)?;
    let mut bytes =
        serde_json::to_vec_pretty(&enriched).map_err(|e| format!("cannot encode output: {e}"))?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| format!("cannot create new output {}: {e}", output.display()))?;
    file.write_all(&bytes)
        .map_err(|e| format!("cannot write output {}: {e}", output.display()))?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("mutation-plan: {error}");
        std::process::exit(2);
    }
}
