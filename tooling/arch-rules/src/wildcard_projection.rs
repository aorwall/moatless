//! Production SQL names the columns it projects.
//!
//! A cached `SELECT *` or `RETURNING *` plan fails with `cached plan must not
//! change result type` on every replica still running when a migration adds or
//! drops a column; `crates/tasks/src/task/access_dao.rs` shows the fix.

use crate::sql::pattern;
use crate::workspace::Workspace;
use crate::{Violation, annotations, sql};
use anyhow::Result;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use syn::spanned::Spanned;

pub const RULE: &str = "wildcard-projection";

/// Whether one projection list, the text after `SELECT` or `RETURNING`,
/// names `*` or `alias.*`. `count(*)` is not a projection wildcard.
fn wildcard(list: &str) -> bool {
    static WILDCARD: OnceLock<Regex> = OnceLock::new();
    pattern(
        &WILDCARD,
        r"(?is)^\s*(?:distinct\s+(?:on\s*\([^)]*\)\s*)?|all\s+)?\*|,\s*\*|\b[a-z_][a-z0-9_]*\s*\.\s*\*",
    )
    .is_match(list)
}

/// The wildcard forms one SQL literal uses, as `SELECT *` or `RETURNING *`.
///
/// A select list ends at the next `FROM`, so a wildcard after a projected
/// subquery is missed rather than a `FROM` clause misread as a projection.
fn wildcards(literal: &str) -> Vec<&'static str> {
    static CLAUSE: OnceLock<Regex> = OnceLock::new();
    static FROM: OnceLock<Regex> = OnceLock::new();
    let sql = sql::uncomment(literal);
    let from = pattern(&FROM, r"(?i)\bfrom\b|;");
    let mut found = Vec::new();
    for c in pattern(&CLAUSE, r"(?i)\b(select|returning)\b").captures_iter(&sql) {
        let rest = &sql[c.get(0).map_or(0, |m| m.end())..];
        let list = &rest[..from.find(rest).map_or(rest.len(), |m| m.start())];
        if wildcard(list) {
            found.push(if c[1].eq_ignore_ascii_case("select") {
                "SELECT *"
            } else {
                "RETURNING *"
            });
        }
    }
    found
}

/// The literals of an item, split per `impl` method so a key names the method;
/// an `impl`'s consts and macros share the `impl`'s own scope.
fn scopes(item: &syn::Item) -> Vec<(Option<String>, usize, Vec<String>)> {
    match item {
        syn::Item::Impl(i) => {
            let mut out: Vec<_> = i
                .items
                .iter()
                .filter_map(|m| match m {
                    syn::ImplItem::Fn(f) if !crate::source_index::test_only(&f.attrs) => Some((
                        Some(f.sig.ident.to_string()),
                        f.span().start().line,
                        sql::impl_item_literals(m),
                    )),
                    _ => None,
                })
                .collect();
            let shared: Vec<String> = i
                .items
                .iter()
                .filter(|m| !matches!(m, syn::ImplItem::Fn(_)))
                .flat_map(sql::impl_item_literals)
                .collect();
            if !shared.is_empty() {
                out.push((None, i.span().start().line, shared));
            }
            out
        }
        _ => vec![(None, item.span().start().line, sql::literals(item))],
    }
}

/// One violation per scope and form, keyed with its count so a new wildcard in
/// an already-baselined method is still reported.
fn judge(
    id: &str,
    file: &str,
    scopes: Vec<(Option<String>, usize, Vec<String>)>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    for (method, line, literals) in scopes {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for form in literals.iter().flat_map(|l| wildcards(l)) {
            *counts.entry(form).or_default() += 1;
        }
        let scope = method.map_or_else(|| id.to_string(), |m| format!("{id}::{m}"));
        for (form, count) in counts {
            out.push(Violation::at(
                RULE,
                format!("{scope} uses {form} x{count}"),
                format!("{scope} uses {form}; name the columns so a column migration cannot change a cached plan's result type"),
                format!("{file}:{line}"),
            ));
        }
    }
    out
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for r in &ws.index()?.records {
        if r.test
            || r.file.ends_with("_tests.rs")
            || !r.file.starts_with("crates/")
            || r.file.contains("/tests/")
            || annotations::permits(r, RULE)
        {
            continue;
        }
        out.extend(judge(&r.id, &r.file, scopes(&r.item)));
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;

    #[test]
    fn projection_wildcards_are_found_and_counts_are_not() {
        assert_eq!(wildcards("SELECT * FROM tasks"), ["SELECT *"]);
        assert_eq!(wildcards("select t.*, w.name from tasks t"), ["SELECT *"]);
        assert_eq!(wildcards("SELECT DISTINCT * FROM tasks"), ["SELECT *"]);
        assert_eq!(
            wildcards("UPDATE tasks SET a = 1 RETURNING *"),
            ["RETURNING *"]
        );
        assert_eq!(
            wildcards("WITH x AS (SELECT * FROM a) INSERT INTO b SELECT id FROM x RETURNING b.*"),
            ["SELECT *", "RETURNING *"]
        );
        assert!(wildcards("SELECT count(*) FROM tasks WHERE a = 2 * b").is_empty());
        assert!(wildcards("SELECT id FROM tasks t WHERE t.n > 1 RETURNING id").is_empty());
        assert!(wildcards("-- SELECT * FROM ghost\nSELECT id FROM tasks").is_empty());
        assert!(wildcards("src/**/*.rs").is_empty());
        assert!(wildcards("^moat-.*$").is_empty());
    }

    #[test]
    fn production_wildcards_are_keyed_by_method_and_tests_are_skipped() {
        let mut f = Fixture::new();
        f.member(
            "sample",
            "",
            "mod dao; #[cfg(test)] mod inline { fn t() { let _ = \"SELECT * FROM a\"; } }",
        );
        f.write(
            "sample/src/dao.rs",
            "struct Dao; impl Dao { const FIND: &str = \"SELECT * FROM a\"; const LIST: &str = \"SELECT a.* FROM a\";
                fn find() { sqlx::query(\"SELECT * FROM a\"); sqlx::query(\"SELECT a.* FROM a\"); }
                fn named() { sqlx::query(\"SELECT id FROM a\"); }
                #[cfg(test)] fn fake() { sqlx::query(\"SELECT * FROM a\"); }
            }
            const SAVE: &str = \"INSERT INTO a (id) VALUES ($1) RETURNING *\";",
        );
        let ws = f.load();
        let mut found: Vec<String> = ws
            .index()
            .expect("index")
            .records
            .iter()
            .filter(|r| !r.test)
            .flat_map(|r| judge(&r.id, &r.file, scopes(&r.item)))
            .map(|v| v.key)
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "sample::dao::SAVE uses RETURNING * x1",
                "sample::dao::impl  for Dao uses SELECT * x2",
                "sample::dao::impl  for Dao::find uses SELECT * x2",
            ]
        );
    }
}
