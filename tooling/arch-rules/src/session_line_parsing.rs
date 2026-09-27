//! A harness's session line is read only by its `SessionFormat`, under
//! `agent::harness`; the rest of the sandbox reads the neutral content.

use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

pub const RULE: &str = "session-line-parsing";
const OWNER: &str = "moatless_sandbox::agent::harness";
/// Keys and line types only a session file carries: Codex rollout item types
/// and Claude session-line fields. Stream-json shares none of them.
const SHAPE: &[&str] = &[
    "response_item",
    "event_msg",
    "turn_context",
    "session_meta",
    "parentUuid",
    "isSidechain",
];

/// String literals an item writes, its attributes' (`#[serde(rename = …)]`)
/// and macro arguments included; a nested module is its own record.
#[derive(Default)]
struct Literals(Vec<String>);
impl<'ast> Visit<'ast> for Literals {
    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        self.0.push(l.value());
    }
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        tokens(&m.tokens, &mut self.0);
    }
    fn visit_meta_list(&mut self, m: &'ast syn::MetaList) {
        tokens(&m.tokens, &mut self.0);
    }
}
fn tokens(stream: &TokenStream, out: &mut Vec<String>) {
    for token in stream.clone() {
        match token {
            TokenTree::Group(g) => tokens(&g.stream(), out),
            TokenTree::Literal(l) => {
                if let Ok(l) = syn::parse_str::<syn::LitStr>(&l.to_string()) {
                    out.push(l.value());
                }
            }
            _ => {}
        }
    }
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for r in &ws.index()?.records {
        if r.test
            || r.package != "moatless-sandbox"
            || r.module == OWNER
            || r.module.starts_with(&format!("{OWNER}::"))
            || annotations::permits(r, RULE)
        {
            continue;
        }
        let mut literals = Literals::default();
        literals.visit_item(&r.item);
        for literal in literals.0 {
            if let Some(key) = SHAPE.iter().find(|key| **key == literal) {
                out.push(Violation::at(
                    RULE,
                    format!("{} -> {key}", r.id),
                    format!(
                        "{} reads a session line's `{key}` outside {OWNER}",
                        r.module
                    ),
                    r.location(),
                ));
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;
    #[test]
    fn only_the_harness_module_names_session_line_shapes() {
        let mut f = Fixture::new();
        f.member("moatless-sandbox", "", "pub mod agent;\npub mod tail;\n");
        f.write("moatless-sandbox/src/agent.rs", "pub mod harness;\n");
        f.write(
            "moatless-sandbox/src/agent/harness.rs",
            r#"pub fn kind(t: &str) -> bool { t == "response_item" }"#,
        );
        f.write(
            "moatless-sandbox/src/tail.rs",
            r#"pub fn kind(t: &str) -> bool { t == "session_meta" || t == "assistant" }
#[derive(serde::Deserialize)]
pub struct Line {
    #[serde(rename = "parentUuid")]
    pub parent: Option<String>,
}"#,
        );
        let found = check(&f.load()).expect("check");
        let keys: Vec<&str> = found.iter().map(|v| v.key.as_str()).collect();
        assert_eq!(keys.len(), 2, "{found:?}");
        assert!(
            keys.iter().any(|k| k.ends_with("session_meta")),
            "{found:?}"
        );
        assert!(keys.iter().any(|k| k.ends_with("parentUuid")), "{found:?}");
    }
}
