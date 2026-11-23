//! Production code names an agent profile or harness through `AgentProfile` or
//! `Harness`, never by its wire id: a string compare compiles on when one is
//! added or renamed.
//!
//! The ids are read from the arms of each type's `as_str`, so the rule never
//! carries its own copy of the list. That file is the owner. An `arch-allow` on
//! an item, or on one `impl` method, excuses a word another vocabulary spells
//! the same, such as the `pi` binary.

use crate::source_index::Record;
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;

pub const RULE: &str = "agent-id-literal";

const OWNERS: &[&str] = &["AgentProfile", "Harness"];

impl Literals {
    /// A macro body is unparsed tokens, so `matches!(p, "codex")` and
    /// `json!({"agent": "codex"})` are read token by token.
    fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
        for tree in tokens {
            match tree {
                proc_macro2::TokenTree::Group(g) => self.tokens(g.stream()),
                proc_macro2::TokenTree::Literal(l) => {
                    if let Ok(s) =
                        syn::parse2::<syn::LitStr>(proc_macro2::TokenTree::Literal(l).into())
                    {
                        self.visit_lit_str(&s);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Every string literal in `item`, with the line it starts on, skipping an
/// `impl` method that carries an exception.
#[derive(Default)]
struct Literals(Vec<(String, usize)>);
impl<'ast> Visit<'ast> for Literals {
    fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
        self.0.push((s.value(), s.span().start().line));
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.tokens(m.tokens.clone());
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if !annotations::excuses(&f.attrs, RULE) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
}

/// The literals `as_str` returns, if `item` is the inherent `impl` of one of
/// the [`OWNERS`].
fn profile_ids(item: &syn::Item) -> Option<BTreeSet<String>> {
    let syn::Item::Impl(imp) = item else {
        return None;
    };
    let syn::Type::Path(ty) = &*imp.self_ty else {
        return None;
    };
    if imp.trait_.is_some() || !OWNERS.iter().any(|o| ty.path.is_ident(o)) {
        return None;
    }
    imp.items.iter().find_map(|i| match i {
        syn::ImplItem::Fn(f) if f.sig.ident == "as_str" => {
            let mut found = Literals::default();
            syn::visit::visit_impl_item_fn(&mut found, f);
            Some(found.0.into_iter().map(|(s, _)| s).collect())
        }
        _ => None,
    })
}

/// One production literal: the module it sits in, its value and where.
struct Use {
    module: String,
    file: String,
    value: String,
    line: usize,
}

/// Judge the gathered literals: one violation per (module, id) outside the
/// files in `owners`.
fn judge(ids: &BTreeSet<String>, owners: &BTreeSet<String>, uses: &[Use]) -> Vec<Violation> {
    let mut found: BTreeMap<(&str, &str), BTreeSet<String>> = BTreeMap::new();
    for u in uses {
        if ids.contains(&u.value) && !owners.contains(&u.file) {
            found
                .entry((&u.module, &u.value))
                .or_default()
                .insert(format!("{}:{}", u.file, u.line));
        }
    }
    found
        .into_iter()
        .map(|((module, id), locations)| Violation {
            rule: RULE,
            key: format!("{module} spells agent id {id}"),
            message: format!(
                "{module} spells the agent id \"{id}\"; use `AgentProfile` or `Harness`, or an `arch-allow` naming the vocabulary that shares the word"
            ),
            locations: locations.into_iter().collect(),
        })
        .collect()
}

fn literals(r: &Record) -> Vec<Use> {
    let mut found = Literals::default();
    found.visit_item(&r.item);
    found
        .0
        .into_iter()
        .map(|(value, line)| Use {
            module: r.module.clone(),
            file: r.file.clone(),
            value,
            line,
        })
        .collect()
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let mut ids = BTreeSet::new();
    let mut owners = BTreeSet::new();
    for r in index.records.iter().filter(|r| !r.test) {
        if let Some(found) = profile_ids(&r.item) {
            ids.extend(found);
            owners.insert(r.file.clone());
        }
    }
    let uses: Vec<Use> = index
        .records
        .iter()
        .filter(|r| !r.test && !annotations::permits(r, RULE))
        .flat_map(literals)
        .collect();
    Ok(judge(&ids, &owners, &uses))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;

    const TYPES: &str = "pub enum AgentProfile { Codex, MiniSwe } \
        impl AgentProfile { pub fn as_str(self) -> &'static str { \
        match self { Self::Codex => \"codex\", Self::MiniSwe => \"mini-swe\" } } } \
        pub enum Harness { Acp } \
        impl Harness { pub fn as_str(self) -> &'static str { \"acp\" } }";

    fn keys(found: &[Violation]) -> Vec<&str> {
        found.iter().map(|v| v.key.as_str()).collect()
    }

    #[test]
    fn a_literal_outside_the_owner_is_flagged_once_per_module_and_id() {
        let mut f = Fixture::new();
        f.member("types", "", TYPES);
        f.member(
            "app",
            "",
            "pub fn a(p: &str) -> bool { p == \"codex\" || matches!(p, \"mini-swe\") } \
             pub fn b(p: &str) -> bool { p == \"codex\" || p == \"acp\" } \
             pub fn c() -> &'static str { \"claude\" }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(
            keys(&found),
            [
                "app spells agent id acp",
                "app spells agent id codex",
                "app spells agent id mini-swe"
            ],
            "{found:?}"
        );
        assert_eq!(found[1].locations.len(), 1, "{found:?}");
    }

    #[test]
    fn tests_and_annotated_items_are_excused() {
        let mut f = Fixture::new();
        f.member("types", "", TYPES);
        f.member(
            "app",
            "",
            "/// arch-allow(agent-id-literal): the CLI binary is also named codex\n\
             pub fn bin() -> &'static str { \"codex\" } \
             pub struct Pi; impl Pi { \
             /// arch-allow(agent-id-literal): the binary is named pi\n\
             fn bin(&self) -> &'static str { \"codex\" } } \
             #[cfg(test)] mod tests { fn t() { let _ = \"mini-swe\"; } }",
        );
        let found = check(&f.load()).expect("check");
        assert!(found.is_empty(), "{found:?}");
    }
}
