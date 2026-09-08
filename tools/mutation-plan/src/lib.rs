// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native-platform applicability for an authoritative cargo-mutants plan.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use proc_macro2::Span;
use quote::ToTokens;
use serde_json::{Value, json};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, ForeignItem, ImplItem, Item, Meta, Pat, Token, TraitItem};

const PLATFORMS: [&str; 3] = ["linux", "windows", "macos"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    line: usize,
    column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Range {
    start: Position,
    end: Position,
}

impl Range {
    fn of(span: Span) -> Self {
        let start = span.start();
        let end = span.end();
        Self {
            start: Position {
                line: start.line,
                column: start.column + 1,
            },
            end: Position {
                line: end.line,
                column: end.column + 1,
            },
        }
    }

    fn contains(self, other: Self) -> bool {
        self.start <= other.start && self.end >= other.end
    }
    fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    fn value(self) -> Value {
        json!({"start":{"line":self.start.line,"column":self.start.column},
               "end":{"line":self.end.line,"column":self.end.column}})
    }

    fn read(value: &Value) -> Result<Self, String> {
        let position = |name: &str| -> Result<Position, String> {
            let number = |field: &str| {
                value[name][field]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("invalid {name}.{field} in source span"))
            };
            Ok(Position {
                line: number("line")?,
                column: number("column")?,
            })
        };
        let range = Self {
            start: position("start")?,
            end: position("end")?,
        };
        if range.start >= range.end {
            return Err("empty or reversed source span".into());
        }
        Ok(range)
    }
}

#[derive(Clone)]
struct RawCfg {
    file: String,
    span: Range,
    meta: Meta,
}

struct Scope {
    span: Range,
    cfg: Vec<RawCfg>,
}
struct Function {
    span: Range,
    body: Range,
    name: String,
}
struct Site {
    span: Range,
    genre: &'static str,
}
struct External {
    span: Range,
    name: String,
    inline_parents: Vec<String>,
    path: Option<String>,
}

#[derive(Default)]
struct Parsed {
    file_cfg: Vec<RawCfg>,
    scopes: Vec<Scope>,
    functions: Vec<Function>,
    sites: Vec<Site>,
    external: Vec<External>,
    unsupported: Vec<(Range, String)>,
}

struct Collector {
    file: String,
    parsed: Parsed,
    inline_parents: Vec<String>,
}

impl Collector {
    fn attributes(&self, attrs: &[Attribute]) -> Vec<RawCfg> {
        attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
            .map(|a| RawCfg {
                file: self.file.clone(),
                span: Range::of(a.span()),
                meta: a.meta.clone(),
            })
            .collect()
    }

    fn scope(&mut self, span: Span, attrs: &[Attribute]) {
        let cfg = self.attributes(attrs);
        if !cfg.is_empty() {
            self.parsed.scopes.push(Scope {
                span: Range::of(span),
                cfg,
            });
        }
    }

    fn site(&mut self, span: Span, genre: &'static str) {
        self.parsed.sites.push(Site {
            span: Range::of(span),
            genre,
        });
    }

    fn function(&mut self, span: Span, body: &syn::Block, name: &syn::Ident) {
        self.parsed.functions.push(Function {
            span: Range::of(span),
            body: Range::of(body.span()),
            name: name.to_string(),
        });
    }
}

macro_rules! attributes_of {
    ($value:expr, $kind:ident, $($variant:ident),+ $(,)?) => {
        match $value { $($kind::$variant(value) => &value.attrs,)+ _ => &[] }
    };
}

macro_rules! scoped_visitors {
    ($(($method:ident, $node:ident)),+ $(,)?) => {$(
        fn $method(&mut self, node: &'ast syn::$node) {
            self.scope(node.span(), &node.attrs);
            visit::$method(self, node);
        }
    )+};
}

impl<'ast> Visit<'ast> for Collector {
    scoped_visitors!(
        (visit_variant, Variant),
        (visit_field, Field),
        (visit_const_param, ConstParam),
        (visit_type_param, TypeParam),
        (visit_lifetime_param, LifetimeParam),
        (visit_bare_fn_arg, BareFnArg),
        (visit_bare_variadic, BareVariadic),
        (visit_receiver, Receiver),
        (visit_variadic, Variadic),
        (visit_field_pat, FieldPat),
        (visit_pat_type, PatType),
    );

    fn visit_foreign_item(&mut self, node: &'ast ForeignItem) {
        self.scope(
            node.span(),
            attributes_of!(node, ForeignItem, Fn, Static, Type, Macro),
        );
        visit::visit_foreign_item(self, node);
    }

    fn visit_pat(&mut self, node: &'ast Pat) {
        self.scope(
            node.span(),
            attributes_of!(
                node,
                Pat,
                Const,
                Ident,
                Lit,
                Macro,
                Or,
                Paren,
                Path,
                Range,
                Reference,
                Rest,
                Slice,
                Struct,
                Tuple,
                TupleStruct,
                Type,
                Wild
            ),
        );
        visit::visit_pat(self, node);
    }

    fn visit_file(&mut self, node: &'ast syn::File) {
        self.parsed.file_cfg = self.attributes(&node.attrs);
        visit::visit_file(self, node);
    }

    fn visit_item(&mut self, node: &'ast Item) {
        self.scope(
            node.span(),
            attributes_of!(
                node,
                Item,
                Const,
                Enum,
                ExternCrate,
                Fn,
                ForeignMod,
                Impl,
                Macro,
                Mod,
                Static,
                Struct,
                Trait,
                TraitAlias,
                Type,
                Union,
                Use
            ),
        );
        if let Item::Verbatim(_) = node {
            self.parsed
                .unsupported
                .push((Range::of(node.span()), "unsupported verbatim item".into()));
        }
        visit::visit_item(self, node);
    }

    fn visit_impl_item(&mut self, node: &'ast syn::ImplItem) {
        self.scope(
            node.span(),
            attributes_of!(node, ImplItem, Const, Fn, Type, Macro),
        );
        visit::visit_impl_item(self, node);
    }

    fn visit_trait_item(&mut self, node: &'ast syn::TraitItem) {
        self.scope(
            node.span(),
            attributes_of!(node, TraitItem, Const, Fn, Type, Macro),
        );
        visit::visit_trait_item(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.function(node.span(), &node.block, &node.sig.ident);
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.function(node.span(), &node.block, &node.sig.ident);
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if let Some(body) = &node.default {
            self.function(node.span(), body, &node.sig.ident);
        }
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if node.content.is_some() {
            if node.attrs.iter().any(|attr| attr.path().is_ident("path")) {
                self.parsed.unsupported.push((
                    Range::of(node.span()),
                    "unsupported inline module path attribute".into(),
                ));
            }
            self.inline_parents.push(node.ident.to_string());
            visit::visit_item_mod(self, node);
            self.inline_parents.pop();
        } else {
            let mut path = None;
            for attr in &node.attrs {
                if attr.path().is_ident("path") {
                    if let Meta::NameValue(value) = &attr.meta
                        && let Expr::Lit(value) = &value.value
                        && let syn::Lit::Str(value) = &value.lit
                        && path.is_none()
                    {
                        path = Some(value.value());
                    } else {
                        self.parsed.unsupported.push((
                            Range::of(node.span()),
                            "unsupported module path attribute".into(),
                        ));
                    }
                }
            }
            self.parsed.external.push(External {
                span: Range::of(node.span()),
                name: node.ident.to_string(),
                inline_parents: self.inline_parents.clone(),
                path,
            });
        }
    }

    fn visit_expr(&mut self, node: &'ast Expr) {
        self.scope(
            node.span(),
            attributes_of!(
                node, Expr, Array, Assign, Async, Await, Binary, Block, Break, Call, Cast, Closure,
                Const, Continue, Field, ForLoop, Group, If, Index, Infer, Let, Lit, Loop, Macro,
                Match, MethodCall, Paren, Path, Range, RawAddr, Reference, Repeat, Return, Struct,
                Try, TryBlock, Tuple, Unary, Unsafe, While, Yield
            ),
        );
        if let Expr::Verbatim(_) = node {
            self.parsed.unsupported.push((
                Range::of(node.span()),
                "unsupported verbatim expression".into(),
            ));
        }
        visit::visit_expr(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        self.site(node.op.span(), "BinaryOperator");
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_unary(&mut self, node: &'ast syn::ExprUnary) {
        self.site(node.op.span(), "UnaryOperator");
        visit::visit_expr_unary(self, node);
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.scope(node.span(), &node.attrs);
        self.site(node.span(), "MatchArm");
        if let Some((_, guard)) = &node.guard {
            self.site(guard.span(), "MatchArmGuard");
        }
        visit::visit_arm(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.scope(node.span(), &node.attrs);
        visit::visit_local(self, node);
    }

    fn visit_field_value(&mut self, node: &'ast syn::FieldValue) {
        self.scope(node.span(), &node.attrs);
        visit::visit_field_value(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.scope(node.span(), &node.attrs);
        visit::visit_stmt_macro(self, node);
    }
}

fn parse(file: &str, source: &str) -> Result<Parsed, String> {
    let syntax =
        syn::parse_file(source).map_err(|e| format!("{file}: cannot parse Rust syntax: {e}"))?;
    let mut collector = Collector {
        file: file.into(),
        parsed: Parsed::default(),
        inline_parents: Vec::new(),
    };
    collector.visit_file(&syntax);
    Ok(collector.parsed)
}

#[derive(Clone)]
struct Condition {
    canonical: String,
    values: [bool; 3],
}

fn metas(meta: &syn::MetaList) -> Result<Vec<Meta>, String> {
    Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(meta.tokens.clone())
        .map(|values| values.into_iter().collect())
        .map_err(|e| format!("invalid cfg syntax: {e}"))
}

fn condition(meta: &Meta) -> Result<Condition, String> {
    let unknown = || format!("unknown cfg predicate: {}", meta.to_token_stream());
    match meta {
        Meta::Path(path) => {
            let (name, values) = if path.is_ident("unix") {
                ("unix", [true, false, true])
            } else if path.is_ident("windows") {
                ("windows", [false, true, false])
            } else if path.is_ident("test") {
                ("test", [false; 3])
            } else {
                return Err(unknown());
            };
            Ok(Condition {
                canonical: name.into(),
                values,
            })
        }
        Meta::NameValue(pair) => {
            let Expr::Lit(value) = &pair.value else {
                return Err(unknown());
            };
            let syn::Lit::Str(value) = &value.lit else {
                return Err(unknown());
            };
            let value = value.value();
            let (name, values) = if pair.path.is_ident("target_os") {
                if !PLATFORMS.contains(&value.as_str()) {
                    return Err(unknown());
                }
                ("target_os", PLATFORMS.map(|platform| platform == value))
            } else if pair.path.is_ident("feature") {
                if !["cli", "fault-injection"].contains(&value.as_str()) {
                    return Err(unknown());
                }
                ("feature", [true; 3])
            } else {
                return Err(unknown());
            };
            Ok(Condition {
                canonical: format!("{name} = {}", json!(value)),
                values,
            })
        }
        Meta::List(list) => {
            let parts = metas(list)?
                .iter()
                .map(condition)
                .collect::<Result<Vec<_>, _>>()?;
            let (name, values) = if list.path.is_ident("all") {
                (
                    "all",
                    std::array::from_fn(|i| parts.iter().all(|p| p.values[i])),
                )
            } else if list.path.is_ident("any") {
                (
                    "any",
                    std::array::from_fn(|i| parts.iter().any(|p| p.values[i])),
                )
            } else if list.path.is_ident("not") && parts.len() == 1 {
                ("not", parts[0].values.map(|v| !v))
            } else {
                return Err(unknown());
            };
            Ok(Condition {
                canonical: format!(
                    "{name}({})",
                    parts
                        .iter()
                        .map(|p| p.canonical.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                values,
            })
        }
    }
}

fn attribute_condition(meta: &Meta) -> Result<Option<Condition>, String> {
    let Meta::List(list) = meta else {
        return Err("invalid cfg attribute".into());
    };
    if list.path.is_ident("cfg") {
        let parts = metas(list)?;
        if parts.len() != 1 {
            return Err("cfg requires one predicate".into());
        }
        return condition(&parts[0]).map(Some);
    }
    if list.path.is_ident("cfg_attr") {
        let parts = metas(list)?;
        if parts.len() < 2 {
            return Err("cfg_attr requires a predicate and attribute".into());
        }
        let when = condition(&parts[0])?;
        let mut applied = Vec::new();
        for inner in &parts[1..] {
            if inner.path().is_ident("cfg") || inner.path().is_ident("cfg_attr") {
                if let Some(predicate) = attribute_condition(inner)? {
                    applied.push(predicate);
                }
            } else if inner.path().is_ident("path") {
                return Err("unsupported cfg_attr changing module paths".into());
            }
        }
        if applied.is_empty() {
            return Ok(None);
        }
        let values =
            std::array::from_fn(|i| !when.values[i] || applied.iter().all(|p| p.values[i]));
        return Ok(Some(Condition {
            canonical: format!(
                "any(not({}), all({}))",
                when.canonical,
                applied
                    .iter()
                    .map(|p| p.canonical.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            values,
        }));
    }
    Err("unsupported configuration attribute".into())
}

fn cfg_for(parsed: &Parsed, span: Range, strict_overlap: bool) -> Result<Vec<RawCfg>, String> {
    let mut result = parsed.file_cfg.clone();
    for scope in &parsed.scopes {
        if scope.span.contains(span) {
            result.extend(scope.cfg.clone());
        } else if strict_overlap && scope.span.intersects(span) {
            return Err("ambiguous mutation overlaps a nested cfg boundary".into());
        }
    }
    for (unsupported, reason) in &parsed.unsupported {
        if unsupported.contains(span) || unsupported.intersects(span) {
            return Err(reason.clone());
        }
    }
    Ok(result)
}

struct Source {
    text: String,
    syntax: Parsed,
    inherited: Vec<RawCfg>,
}

struct Sources {
    root: PathBuf,
    files: BTreeMap<String, Source>,
    active: BTreeSet<String>,
}

impl Sources {
    fn name(&self, path: &Path) -> Result<String, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| format!("source path escapes root: {}", path.display()))?;
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    fn load(
        &mut self,
        path: &Path,
        inherited: Vec<RawCfg>,
        crate_root: bool,
    ) -> Result<(), String> {
        let name = self.name(path)?;
        if self.active.contains(&name) {
            return Err(format!("cyclic module ownership: {name}"));
        }
        if self.files.contains_key(&name) {
            return Err(format!("ambiguous module ownership: {name}"));
        }
        if self.active.len() >= 64 {
            return Err("module nesting exceeds 64 levels".into());
        }
        self.active.insert(name.clone());
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {name}: {e}"))?;
        let syntax = parse(&name, &text)?;
        let parent = path.parent().ok_or("source file has no parent")?;
        let base = if crate_root || path.file_name().is_some_and(|n| n == "mod.rs") {
            parent.to_owned()
        } else {
            parent.join(path.file_stem().ok_or("source file has no stem")?)
        };
        for external in &syntax.external {
            let mut context = inherited.clone();
            context.extend(cfg_for(&syntax, external.span, false)?);
            // An explicit path outside inline modules is relative to the
            // containing source file, not its implicit module directory.
            let mut directory = if external.path.is_some() && external.inline_parents.is_empty() {
                parent.to_owned()
            } else {
                base.clone()
            };
            directory.extend(&external.inline_parents);
            let choices = if let Some(explicit) = &external.path {
                vec![directory.join(explicit)]
            } else {
                vec![
                    directory.join(format!("{}.rs", external.name)),
                    directory.join(&external.name).join("mod.rs"),
                ]
            };
            let existing: Vec<_> = choices.iter().filter(|p| p.is_file()).collect();
            match existing.as_slice() {
                [one] => self.load(one, context, false)?,
                [] => {
                    // An absent test-only module cannot own production candidates.
                    let mut active = [true; 3];
                    for cfg in &context {
                        if let Some(predicate) = attribute_condition(&cfg.meta)? {
                            for (enabled, value) in active.iter_mut().zip(predicate.values) {
                                *enabled &= value;
                            }
                        }
                    }
                    if active.iter().any(|v| *v) {
                        return Err(format!("unresolved module {} in {name}", external.name));
                    }
                }
                _ => {
                    return Err(format!(
                        "ambiguous module files for {} in {name}",
                        external.name
                    ));
                }
            }
        }
        self.active.remove(&name);
        self.files.insert(
            name,
            Source {
                text,
                syntax,
                inherited,
            },
        );
        Ok(())
    }
}

fn validate_span(text: &str, span: Range) -> Result<(), String> {
    let lines: Vec<_> = text.lines().collect();
    for point in [span.start, span.end] {
        let line = lines
            .get(point.line - 1)
            .ok_or("source span names a missing line")?;
        if point.column > line.chars().count() + 1 {
            return Err("source span names a missing column".into());
        }
    }
    Ok(())
}

/// Retains every candidate and adds platform applicability from source syntax.
///
/// The supported execution configuration enables `cli` and `fault-injection`,
/// with `test=false`. The first applicable platform in Linux, Windows, macOS
/// order owns execution; every applicable platform is retained in the record.
///
/// # Errors
///
/// Returns an error for malformed input, duplicate names, unsupported cfg or
/// mutation syntax, ambiguous module/function ownership, mismatched source
/// spans, and candidates with no applicable platform. No partial plan is returned.
pub fn enrich(root: &Path, input: Value) -> Result<Value, String> {
    let input = input
        .as_array()
        .ok_or("input must be an authoritative mutant array")?;
    let mut ids = BTreeSet::new();
    for row in input {
        let name = row["name"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("candidate has no name")?;
        if !ids.insert(name) {
            return Err(format!("duplicate candidate name: {name}"));
        }
        if row.get("applicability").is_some() {
            return Err(format!("{name}: applicability already exists"));
        }
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve source root: {e}"))?;
    let mut sources = Sources {
        root: root.clone(),
        files: BTreeMap::new(),
        active: BTreeSet::new(),
    };
    for name in ["src/lib.rs", "src/main.rs"] {
        let path = root.join(name);
        if path.is_file() {
            sources.load(&path, Vec::new(), true)?;
        }
    }
    if sources.files.is_empty() {
        return Err("source root has no src/lib.rs or src/main.rs".into());
    }
    let mut output = Vec::with_capacity(input.len());
    for row in input {
        let name = row["name"].as_str().ok_or("candidate has no name")?;
        let enrich_one = || -> Result<Value, String> {
            let filename = row["file"].as_str().ok_or("candidate has no source file")?;
            if filename.contains('\\')
                || Path::new(filename)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err("candidate file must be a normalized relative path".into());
            }
            let source = sources
                .files
                .get(filename)
                .ok_or("candidate source has no unambiguous crate module owner")?;
            let span = Range::read(&row["span"])?;
            validate_span(&source.text, span)?;
            let genre = row["genre"]
                .as_str()
                .ok_or("candidate has no mutation genre")?;
            let owner = if row["function"].is_object() {
                let expected = Range::read(&row["function"]["span"])?;
                let found: Vec<_> = source
                    .syntax
                    .functions
                    .iter()
                    .filter(|f| f.span == expected)
                    .collect();
                let [one] = found.as_slice() else {
                    return Err("function owner span does not identify one source function".into());
                };
                if !one.body.contains(span) {
                    return Err("mutation span is outside the declared function body".into());
                }
                let declared = row["function"]["function_name"]
                    .as_str()
                    .ok_or("function owner has no name")?;
                if declared != one.name && !declared.ends_with(&format!("::{}", one.name)) {
                    return Err("function owner name does not match source syntax".into());
                }
                Some(*one)
            } else {
                None
            };
            let selection = if genre == "FnValue" {
                owner
                    .ok_or("FnValue requires an unambiguous function owner")?
                    .span
            } else {
                if ![
                    "BinaryOperator",
                    "UnaryOperator",
                    "MatchArm",
                    "MatchArmGuard",
                ]
                .contains(&genre)
                {
                    return Err(format!("unsupported mutation genre: {genre}"));
                }
                let sites = source
                    .syntax
                    .sites
                    .iter()
                    .filter(|s| s.genre == genre && s.span == span)
                    .count();
                if sites != 1 {
                    return Err(
                        "mutation span does not identify one matching Rust syntax node".into(),
                    );
                }
                span
            };
            let mut cfg = source.inherited.clone();
            cfg.extend(cfg_for(&source.syntax, selection, genre != "FnValue")?);
            let mut active = [true; 3];
            let mut evidence = Vec::new();
            let mut recorded = BTreeSet::new();
            for raw in cfg {
                if !recorded.insert((raw.file.clone(), raw.span.start, raw.span.end)) {
                    continue;
                }
                if let Some(predicate) = attribute_condition(&raw.meta)? {
                    for (enabled, value) in active.iter_mut().zip(predicate.values) {
                        *enabled &= value;
                    }
                    evidence.push(json!({"file":raw.file,"span":raw.span.value(),"predicate":predicate.canonical}));
                }
            }
            let platforms: Vec<_> = PLATFORMS
                .into_iter()
                .zip(active)
                .filter_map(|(p, active)| active.then_some(p))
                .collect();
            let assigned = platforms
                .first()
                .ok_or("contradictory cfg or no applicable native platform")?;
            let mut enriched = row.clone();
            enriched.as_object_mut().ok_or("candidate must be an object")?.insert("applicability".into(), json!({
                "platforms":platforms, "assigned_platform":assigned,
                "cfg":evidence, "function": owner.map(|function| json!({"file":filename,"span":function.span.value()})),
                "enabled_features":["cli","fault-injection"], "test":false,
            }));
            Ok(enriched)
        };
        output.push(enrich_one().map_err(|error| format!("{name}: {error}"))?);
    }
    Ok(Value::Array(output))
}

#[cfg(test)]
mod tests;
