//! One literal fact has one owning constant.
//!
//! A `const` or `static` whose literal value is also defined in another
//! production crate is a copied fact: nothing fails when one copy changes. Only
//! string, number and array literals count, and test-only items do not. A
//! number counts only under the same name, since `2_000` alone is no fact.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use syn::visit::Visit;

use crate::Violation;
use crate::annotations;
use crate::workspace::Workspace;

pub const RULE: &str = "duplicate-constant-value";

/// A string shorter than this collides by coincidence (`"id"`, `"GET"`).
const MIN_STR_LEN: usize = 6;

/// An integer below this is a retry count or a small timeout, and two crates
/// choosing `MAX_ATTEMPTS = 5` apart is coincidence.
const MIN_INT: u128 = 100;

/// Values each defining crate means something different by, checked against
/// the repo: an entry no longer defined in two crates is reported as stale.
const COINCIDENTAL: &[&str] = &[
    // An app-registry key, a GitHub App key and a model-choice sentinel.
    "\"default\"",
    // A workspace-sync format label and a model-name placeholder.
    "\"unknown\"",
    // A Compose project, a Docker label value and a T3 environment name.
    "\"moatless\"",
    // A secret audit scope and a restic snapshot label.
    "\"workspace\"",
    // A Codex model-provider id and a T3 usage-limit source id.
    "\"cliproxy\"",
];

/// One constant: its crate, its location, its name and its normalised value.
#[derive(Clone)]
struct Constant {
    package: String,
    location: String,
    name: String,
    value: String,
}

impl Constant {
    /// What two copies of one fact share: the value, and for a number the name.
    fn identity(&self) -> String {
        if self.value.starts_with(['"', '[']) {
            self.value.clone()
        } else {
            format!("{} = {}", self.name, self.value)
        }
    }
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let production: BTreeSet<&str> = ws
        .members()
        .filter(|m| m.dir.starts_with(ws.root.join("crates")))
        .map(|m| m.name.as_str())
        .collect();
    let mut found = Vec::new();
    for r in index.records.iter().filter(|r| {
        !r.test && production.contains(r.package.as_str()) && !annotations::permits(r, RULE)
    }) {
        // An inline module's items are records of their own.
        if matches!(r.item, syn::Item::Mod(_)) {
            continue;
        }
        let mut consts = Consts::default();
        consts.visit_item(&r.item);
        found.extend(
            consts
                .found
                .into_iter()
                .map(|(line, name, value)| Constant {
                    package: r.package.clone(),
                    location: format!("{}:{line}", r.file),
                    name,
                    value,
                }),
        );
    }
    Ok(duplicates(&found, COINCIDENTAL))
}

/// Every production-code `const`/`static` in one item, nested ones included,
/// as (line, name, value).
#[derive(Default)]
struct Consts {
    found: Vec<(usize, String, String)>,
}

impl Consts {
    fn push(&mut self, attrs: &[syn::Attribute], ident: &syn::Ident, expr: &syn::Expr) {
        if crate::source_index::test_only(attrs) {
            return;
        }
        if let Some(value) = literal(expr).filter(|v| significant(v)) {
            self.found
                .push((ident.span().start().line, ident.to_string(), value));
        }
    }
}

impl<'ast> Visit<'ast> for Consts {
    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.push(&c.attrs, &c.ident, &c.expr);
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.push(&s.attrs, &s.ident, &s.expr);
    }

    fn visit_impl_item_const(&mut self, c: &'ast syn::ImplItemConst) {
        self.push(&c.attrs, &c.ident, &c.expr);
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if !crate::source_index::test_only(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if !crate::source_index::test_only(&f.attrs) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
}

/// The value of a string, integer, float or array-of-literals expression, in
/// a form where equal values are equal strings; `None` for anything computed.
fn literal(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(l) => match &l.lit {
            syn::Lit::Str(s) => Some(format!("{:?}", s.value())),
            syn::Lit::Int(i) => i.base10_parse::<u128>().ok().map(|n| n.to_string()),
            syn::Lit::Float(f) => f.base10_parse::<f64>().ok().map(|n| format!("{n:?}")),
            _ => None,
        },
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => literal(expr).map(|v| format!("-{v}")),
        syn::Expr::Reference(r) => literal(&r.expr),
        syn::Expr::Paren(p) => literal(&p.expr),
        syn::Expr::Array(a) => {
            let items = a.elems.iter().map(literal).collect::<Option<Vec<_>>>()?;
            Some(format!("[{}]", items.join(", ")))
        }
        _ => None,
    }
}

/// Whether a value is specific enough that a second copy is a copied fact.
fn significant(value: &str) -> bool {
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return inner.chars().count() >= MIN_STR_LEN;
    }
    if value.starts_with('[') {
        return value != "[]";
    }
    value
        .trim_start_matches('-')
        .parse::<u128>()
        .ok()
        .is_none_or(|n| n >= MIN_INT)
}

/// Identities defined in more than one crate, and every `ignored` entry that no
/// longer is.
fn duplicates(found: &[Constant], ignored: &[&str]) -> Vec<Violation> {
    let mut defined: BTreeMap<String, BTreeMap<&str, BTreeSet<&str>>> = BTreeMap::new();
    for c in found {
        defined
            .entry(c.identity())
            .or_default()
            .entry(&c.package)
            .or_default()
            .insert(&c.location);
    }
    let mut out: Vec<Violation> = ignored
        .iter()
        .filter(|v| defined.get(**v).is_none_or(|crates| crates.len() < 2))
        .map(|v| Violation {
            rule: RULE,
            key: format!("stale ignore {v}"),
            message: format!("{v} is no longer defined in two crates; drop it from `COINCIDENTAL`"),
            locations: vec!["tooling/arch-rules/src/duplicate_constant_value.rs".to_string()],
        })
        .collect();
    out.extend(
        defined
            .into_iter()
            .filter(|(identity, crates)| crates.len() > 1 && !ignored.contains(&identity.as_str()))
            .map(|(identity, crates)| {
                let where_ = crates.keys().copied().collect::<Vec<_>>().join(", ");
                Violation {
                    rule: RULE,
                    key: format!("{identity} in {where_}"),
                    message: format!(
                        "{identity} is defined in {} crates: {where_}; give it one owner",
                        crates.len()
                    ),
                    locations: crates.values().flatten().map(ToString::to_string).collect(),
                }
            }),
    );
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "source fixtures must parse")]
mod tests {
    use super::*;

    fn constant(package: &str, name: &str, value: &str) -> Constant {
        Constant {
            package: package.to_string(),
            location: format!("crates/{package}/src/lib.rs:1"),
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn parsed(src: &str) -> Option<String> {
        let item: syn::ItemConst = syn::parse_str(src).ok()?;
        literal(&item.expr)
    }

    #[test]
    fn a_value_in_one_crate_is_not_a_duplicate() {
        let found = duplicates(
            &[
                constant("moatless-core", "A", "\"CODEX_AUTH_JSON\""),
                constant("moatless-core", "B", "\"CODEX_AUTH_JSON\""),
            ],
            &[],
        );
        assert!(found.is_empty());
    }

    #[test]
    fn a_string_in_two_crates_is_keyed_by_value_and_crates_under_any_name() {
        let found = duplicates(
            &[
                constant("moatless-core", "CODEX_AUTH_JSON", "\"CODEX_AUTH_JSON\""),
                constant("moatless-sandbox", "CODEX_AUTH_KEY", "\"CODEX_AUTH_JSON\""),
            ],
            &[],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].key,
            "\"CODEX_AUTH_JSON\" in moatless-core, moatless-sandbox"
        );
        assert_eq!(found[0].locations.len(), 2);
    }

    #[test]
    fn a_number_is_a_duplicate_only_under_one_name() {
        let found = duplicates(
            &[
                constant("moatless-core", "RESUME_WINDOW", "5000"),
                constant("moatless-t3code", "RESUME_WINDOW", "5000"),
                constant("moatless-tasks", "MAX_NOTE", "5000"),
            ],
            &[],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].key,
            "RESUME_WINDOW = 5000 in moatless-core, moatless-t3code"
        );
    }

    #[test]
    fn a_new_participant_changes_the_key() {
        let two = [
            constant("moatless-core", "A", "\"tool_response\""),
            constant("moatless-t3code", "B", "\"tool_response\""),
        ];
        let mut three = two.to_vec();
        three.push(constant("moatless-tasks", "C", "\"tool_response\""));
        assert_ne!(
            duplicates(&two, &[])[0].id(),
            duplicates(&three, &[])[0].id()
        );
    }

    #[test]
    fn an_ignored_value_is_skipped_until_it_goes_stale() {
        let two = [
            constant("moatless-core", "A", "\"default\""),
            constant("moatless-t3code", "B", "\"default\""),
        ];
        assert!(duplicates(&two, &["\"default\""]).is_empty());
        let stale = duplicates(&two[..1], &["\"default\""]);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].key, "stale ignore \"default\"");
    }

    #[test]
    fn equal_values_normalise_to_one_form() {
        assert_eq!(
            parsed("const A: i64 = 5_000;"),
            parsed("const B: u32 = 5000u32;")
        );
        assert_eq!(
            parsed("const A: &[&str] = &[\"a\", \"b\"];"),
            parsed("const B: [&str; 2] = [\"a\", \"b\"];")
        );
        assert_eq!(parsed("const A: i32 = -(1);"), Some("-1".to_string()));
        assert_eq!(parsed("const A: Duration = Duration::from_secs(5);"), None);
    }

    #[test]
    fn short_strings_small_integers_and_empty_arrays_are_coincidence() {
        assert!(!significant("\"GET\""));
        assert!(significant("\"tool_response\""));
        assert!(!significant("[]"));
        assert!(significant("[\"a\"]"));
        assert!(!significant("5"));
        assert!(significant("5000"));
    }

    #[test]
    fn nested_and_test_only_constants() {
        let item: syn::Item = syn::parse_str(
            "impl X { const A: &str = \"nested value\"; fn f() { const B: u64 = 9000; } \
             #[cfg(test)] fn g() { const C: u64 = 9000; } }",
        )
        .expect("parse");
        let mut consts = Consts::default();
        consts.visit_item(&item);
        let names: Vec<_> = consts.found.into_iter().map(|(_, n, _)| n).collect();
        assert_eq!(names, ["A", "B"]);
    }
}
