//! State assembly and concrete DAO construction stay at their owning roots.

use crate::source_index::{Index, Record};
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::BTreeMap;
use syn::visit::Visit;

pub const RULE: &str = "composition-root";
pub(crate) fn root(file: &str) -> bool {
    file == "crates/server/src/compose.rs"
        || (file.starts_with("crates/server/src/compose/") && !file.ends_with("/mod.rs"))
}
fn state_owner(file: &str) -> bool {
    root(file)
        || matches!(
            file,
            "crates/server/src/state.rs" | "crates/server/src/app.rs"
        )
}
fn router(file: &str) -> bool {
    file.ends_with("/routes.rs")
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
            if p.path.segments.last().is_some_and(|s| s.ident == "new")
                && let Some(q) = &p.qself
                && let syn::Type::Path(p) = &*q.ty
            {
                self.paths.push(
                    p.path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect(),
                );
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
}
fn owner_composition(file: &str) -> bool {
    file.ends_with("/compose.rs") || (file.contains("/compose/") && !file.ends_with("/mod.rs"))
}

fn construction_allowed(file: &str, package: &str, dao: &str) -> bool {
    file.ends_with("/dao.rs")
        || file.contains("/dao/")
        || root(file)
        || (owner_composition(file)
            && dao.split("::").next() == Some(package.replace('-', "_").as_str()))
}

fn constructed_daos(index: &Index, r: &Record) -> Vec<String> {
    let mut calls = Constructors::default();
    calls.visit_item(&r.item);
    calls
        .paths
        .iter()
        .filter_map(|p| {
            let resolved = index.resolve(r, p).unwrap_or_else(|| p.join("::"));
            let is_dao = {
                let mut parts = resolved.rsplit("::");
                let last = parts.next().unwrap_or_default();
                (last.starts_with("Pg") && last.ends_with("Dao"))
                    || (last == "new"
                        && parts
                            .next()
                            .is_some_and(|s| s.starts_with("Pg") && s.ends_with("Dao")))
            };
            is_dao.then_some(resolved)
        })
        .collect()
}
pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
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
                || r.references.iter().any(|p| {
                    index
                        .resolve(r, p)
                        .is_some_and(|p| p.ends_with("::AppState"))
                });
            if names_state && !state_owner(file) && (!router(file) || handler) {
                out.push(Violation::at(RULE, format!("{} names AppState", r.id), "AppState belongs to state assembly; a routing exemption cannot include a handler", r.location()));
            }
            if constructed_daos(index, r)
                .iter()
                .any(|dao| !construction_allowed(file, &r.package, dao))
            {
                out.push(Violation::at(RULE, format!("{} constructs a PgDao", r.id), "construct concrete DAOs in their owner's assembly or DAO implementation, or in backend composition", r.location()));
            }
        }
    }
    Ok(out)
}
#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;

    #[test]
    fn owner_assembly_does_not_allow_foreign_daos_or_app_state() {
        for file in [
            "crates/identity/src/auth/compose.rs",
            "crates/identity/src/auth/compose/sessions.rs",
        ] {
            assert!(construction_allowed(
                file,
                "moatless-identity",
                "moatless_identity::auth::sessions::PgSessionDao::new"
            ));
            assert!(!construction_allowed(
                file,
                "moatless-identity",
                "moatless_tasks::task::PgTaskDao::new"
            ));
            assert!(!state_owner(file));
        }
        for file in [
            "crates/identity/src/auth/handlers.rs",
            "crates/identity/src/auth/compose/mod.rs",
        ] {
            assert!(!construction_allowed(
                file,
                "moatless-identity",
                "moatless_identity::auth::sessions::PgSessionDao"
            ));
        }
    }

    #[test]
    fn assembly_resolves_constructor_owners_through_aliases() {
        let mut f = Fixture::new();
        f.member(
            "owner",
            "",
            "pub mod dao; pub use dao::PgSessionDao as Exported; mod compose; mod handlers;",
        );
        f.write(
            "owner/src/dao.rs",
            "pub struct PgSessionDao; impl PgSessionDao { pub fn new() -> Self { Self } }",
        );
        f.write(
            "owner/src/compose.rs",
            "use crate::Exported as Local; fn build() { Local::new(); }",
        );
        f.write(
            "owner/src/handlers.rs",
            "use crate::Exported as Local; fn handle() { Local::new(); }",
        );
        f.member(
            "consumer",
            "renamed = { package = 'owner', path = '../owner' }",
            "mod compose;",
        );
        f.write(
            "consumer/src/compose.rs",
            "use renamed::Exported as Foreign; fn build() { Foreign::new(); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found
                .iter()
                .any(|v| v.locations[0].contains("owner/src/handlers.rs:"))
        );
        assert!(
            found
                .iter()
                .any(|v| v.locations[0].contains("consumer/src/compose.rs:"))
        );
    }

    #[test]
    fn assembly_module_roots_remain_declaration_only() {
        let mut f = Fixture::new();
        f.member("owner", "", "mod compose;");
        f.write("owner/src/compose/mod.rs", "fn build() {}");
        let found = crate::declaration_only_roots::check(&f.load()).expect("check");
        assert_eq!(found.len(), 1, "{found:?}");
    }
    #[test]
    fn nested_modules_are_not_composition_roots() {
        assert!(root("crates/server/src/compose.rs"));
        assert!(root("crates/server/src/compose/stores.rs"));
        assert!(!root("crates/server/src/compose/mod.rs"));
        assert!(!root("crates/server/src/state.rs"));
        assert!(state_owner("crates/server/src/state.rs"));
        assert!(!root("crates/server/src/task/compose.rs"));
        assert!(router("crates/loops/src/bindings/routes.rs"));
    }
}
