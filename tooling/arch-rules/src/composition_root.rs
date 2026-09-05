//! State assembly and concrete DAO construction stay at their owning roots.

use crate::source_index::{Index, Record};
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::BTreeMap;
use syn::visit::Visit;

pub const RULE: &str = "composition-root";
fn root(file: &str) -> bool {
    [
        "main.rs",
        "state.rs",
        "compose.rs",
        "adapter_installations.rs",
    ]
    .iter()
    .any(|name| file == format!("crates/server/src/{name}"))
}
fn router(file: &str) -> bool {
    file.ends_with("/routes.rs") || file.ends_with("/loops/http/mod.rs")
}
#[derive(Default)]
struct Constructors {
    paths: Vec<Vec<String>>,
}
impl<'ast> Visit<'ast> for Constructors {
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            self.paths.push(
                p.path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect(),
            );
            if p.path.segments.last().is_some_and(|s| s.ident == "new") {
                if let Some(q) = &p.qself {
                    if let syn::Type::Path(p) = &*q.ty {
                        self.paths.push(
                            p.path
                                .segments
                                .iter()
                                .map(|s| s.ident.to_string())
                                .collect(),
                        );
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
}
fn constructs(index: &Index, r: &Record) -> bool {
    let mut calls = Constructors::default();
    calls.visit_item(&r.item);
    calls.paths.iter().any(|p| {
        let resolved = index.resolve(r, p).unwrap_or_else(|| p.join("::"));
        {
            let mut parts = resolved.rsplit("::");
            let last = parts.next().unwrap_or_default();
            (last.starts_with("Pg") && last.ends_with("Dao"))
                || (last == "new"
                    && parts
                        .next()
                        .is_some_and(|s| s.starts_with("Pg") && s.ends_with("Dao")))
        }
    })
}
pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let mut files: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    for r in index.records.iter().filter(|r| !r.test) {
        files.entry(&r.file).or_default().push(r);
    }
    let mut out = Vec::new();
    for (file, records) in files {
        let handler = records.iter().any(|r| {
            matches!(&r.item, syn::Item::Fn(f) if f.sig.asyncness.is_some())
                || r.references
                    .iter()
                    .any(|p| p.last().is_some_and(|s| s == "State"))
        });
        for r in records {
            if annotations::permits(r, RULE) {
                continue;
            }
            let names_state = r
                .references
                .iter()
                .any(|p| p.last().is_some_and(|s| s == "AppState"))
                || index.paths(r).any(|p| p.ends_with("::AppState"));
            if names_state && !root(file) && (!router(file) || handler) {
                out.push(Violation::at(RULE, format!("{} names AppState", r.id), "AppState belongs to state assembly; a routing exemption cannot include a handler", file));
            }
            if !file.ends_with("/dao.rs")
                && !root(file)
                && !file.ends_with("/secret/cli.rs")
                && constructs(index, r)
            {
                out.push(Violation::at(RULE, format!("{} constructs a PgDao", r.id), "construct concrete DAOs at the composition root or in their owning DAO implementation", file));
            }
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_modules_are_not_composition_roots() {
        assert!(root("crates/server/src/compose.rs"));
        assert!(!root("crates/server/src/task/compose.rs"));
        assert!(router("crates/loops/src/bindings/routes.rs"));
    }
}
