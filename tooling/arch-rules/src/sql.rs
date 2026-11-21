//! SQL literals and table operations used by ownership checks.

use proc_macro2::{TokenStream, TokenTree};
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::OnceLock;
use syn::visit::Visit;

#[derive(Default)]
struct Literals {
    values: Vec<String>,
}
impl<'ast> Visit<'ast> for Literals {
    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        self.values.push(l.value());
    }
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        tokens(&m.tokens, &mut self.values);
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
/// String literals written by an item, including macro arguments.
pub fn literals(item: &syn::Item) -> Vec<String> {
    let mut found = Literals::default();
    found.visit_item(item);
    found.values
}
fn pattern(cell: &'static OnceLock<Regex>, expression: &str) -> &'static Regex {
    cell.get_or_init(|| {
        Regex::new(expression).unwrap_or_else(|e| panic!("invalid SQL matcher: {e}"))
    })
}
pub(crate) fn uncomment(sql: &str) -> String {
    static COMMENTS: OnceLock<Regex> = OnceLock::new();
    pattern(&COMMENTS, r"(?s)/\*.*?\*/|--[^\n]*")
        .replace_all(sql, " ")
        .into_owned()
}
/// Tables read or written by a SQL literal, paired with its write flag.
pub fn tables(sql: &str) -> BTreeSet<(String, bool)> {
    static TABLE: OnceLock<Regex> = OnceLock::new();
    let sql = uncomment(sql);
    pattern(&TABLE, r#"(?i)\b(delete\s+from|from|join|into|update|truncate(?:\s+table)?)\s+(?:only\s+)?(?:[a-z_][a-z0-9_]*\.)?"?([a-z_][a-z0-9_]*)"?"#)
        .captures_iter(&sql).map(|c| (c[2].to_lowercase(), { let operation = c[1].to_lowercase(); matches!(operation.as_str(), "into" | "update") || operation.starts_with("delete") || operation.starts_with("truncate") })).collect()
}
/// Replay table operations in a migration in source order.
pub fn migrate(live: &mut BTreeSet<String>, sql: &str) {
    static DDL: OnceLock<Regex> = OnceLock::new();
    let sql = uncomment(sql);
    for c in pattern(&DDL, r#"(?i)\b(create|drop|alter)\s+table\s+(?:if\s+(?:not\s+)?exists\s+)?(?:[a-z_][a-z0-9_]*\.)?"?([a-z_][a-z0-9_]*)"?(?:\s+rename\s+to\s+"?([a-z_][a-z0-9_]*)"?)?"#).captures_iter(&sql) {
        let table = c[2].to_lowercase();
        match c[1].to_lowercase().as_str() {
            "create" => { live.insert(table); }
            "drop" => { live.remove(&table); }
            "alter" => { if let Some(to) = c.get(3) { live.remove(&table); live.insert(to.as_str().to_lowercase()); } }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_order_and_comments_are_respected() {
        let mut live = BTreeSet::new();
        migrate(
            &mut live,
            "-- CREATE TABLE ghost ();\nCREATE TABLE old_name (); ALTER TABLE old_name RENAME TO next_name; DROP TABLE next_name; CREATE TABLE actual ();",
        );
        assert_eq!(live, BTreeSet::from(["actual".to_string()]));
    }
    #[test]
    fn joins_and_quoted_schema_tables_are_found() {
        assert_eq!(
            tables("SELECT * FROM public.\"tasks\" JOIN workspaces w ON true -- JOIN ghost"),
            BTreeSet::from([
                ("tasks".to_string(), false),
                ("workspaces".to_string(), false)
            ])
        );
        assert!(
            tables("WITH chosen AS (SELECT * FROM tasks) DELETE FROM workspaces")
                .contains(&("workspaces".to_string(), true))
        );
        assert!(tables("DELETE FROM tasks").contains(&("tasks".to_string(), true)));
    }
}
