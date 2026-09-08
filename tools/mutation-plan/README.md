<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Native mutation applicability

This standalone, unpublished workspace annotates the complete `cargo-mutants 27.1.0`
candidate array. It uses `syn` to locate the real Rust expression or function and its
enclosing configuration attributes. It never deletes an inapplicable candidate or
interprets an inactive-platform outcome as caught.

```console
cargo build --locked --offline --manifest-path tools/mutation-plan/Cargo.toml
tools/mutation-plan/target/debug/ginary-mutation-plan \
  --source-root . --input authoritative-plan.json --output enriched-plan.json
```

On Windows the executable has the `.exe` suffix. A configured `CARGO_TARGET_DIR`
changes its location. The output must be a new file: existing evidence is refused.
Input is the array produced by `cargo mutants --list --json --diff`, with no platform
exclusions. The original fields and order are preserved, adding only:

```json
{
  "applicability": {
    "platforms": ["linux", "macos"],
    "assigned_platform": "linux",
    "cfg": [{"file": "src/trailer.rs", "span": {}, "predicate": "unix"}],
    "function": {"file": "src/trailer.rs", "span": {}},
    "enabled_features": ["cli", "fault-injection"],
    "test": false
  }
}
```

The abbreviated span objects above are complete start/end line-column objects in
real output. Both coordinates are one-based; columns count Unicode characters,
matching cargo-mutants/proc-macro2. The AST handles comments, raw strings, CRLF,
inline modules, external modules, impls, traits, functions, fields, variants, generic
parameters, locals, match arms and attributed expressions. Parent module conditions from `src/lib.rs` and `src/main.rs`
are inherited. Multiple source owners, missing active modules and source paths
outside the root are errors.

Supported predicates are `unix`, `windows`, `test`, `target_os` values `linux`,
`windows`, `macos`, and the enabled features `cli` and `fault-injection`; `all`,
`any` and `not` compose them. `cfg_attr` containing `cfg` has implication semantics.
Explicit external module paths follow the [Rust Reference's source-relative rules](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute).
Inline module path overrides and conditional module paths are explicitly unsupported,
and fail closed instead of selecting a possibly unrelated file.
Unknown predicates and candidates with no
applicable platform fail the entire plan. This explicit configuration must be changed
together with its tests if mutation jobs use different features or target families.

For `FnValue`, applicability belongs to the declared function, even if its first
statement is conditional. Binary/unary operators, match arms and match guards must
match exact AST spans. Other mutation genres, unverifiable spans and mutations
partially overlapping a nested conditional boundary are errors. This is intentional:
a future cargo-mutants generation change needs review, rather than silently losing
candidates. Canonical cfg evidence includes the source file and attribute span.

The dispatcher must select the assigned IDs from this complete plan and reconcile
every outcome with them. Regex selection precedes cargo-mutants sharding: when a
dispatcher already selected a canonical shard, applying `--shard` again would split
the subset a second time. This tool does not dispatch jobs, count tests, decide whether
unviable mutants meet a gate, or claim mutation execution has completed.

`cargo test --locked --offline --manifest-path tools/mutation-plan/Cargo.toml` covers
nested cfg inheritance, function-versus-statement ownership, Unicode/CRLF locations,
comment/string traps, external modules, unknown/contradictory predicates, duplicate
IDs and forged spans. The root project's manifest and lock are independent of this tool.
