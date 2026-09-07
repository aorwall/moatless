//! Domain operations do not depend on their HTTP adapters.

use anyhow::Result;

use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "transport-direction";

fn operation(file: &str) -> bool {
    let stem = std::path::Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    matches!(
        stem,
        "service" | "dao" | "creation" | "send" | "closer" | "operations" | "operation"
    ) || file.contains("/operations/")
        || file.contains("/service/")
        || file.contains("/dao/")
}

fn transport(path: &str) -> bool {
    path.contains("::")
        && path
            .split("::")
            .any(|p| matches!(p, "handlers" | "routes" | "axum"))
}

fn adapter(file: &str) -> bool {
    let stem = std::path::Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    matches!(
        stem,
        "lib"
            | "mod"
            | "main"
            | "app"
            | "compose"
            | "state"
            | "routes"
            | "handlers"
            | "error"
            | "schemas"
            | "middleware"
            | "extractor"
            | "startup"
            | "authentication"
    ) || stem.ends_with("_handlers")
        || file.contains("/http/")
        || file.contains("/transport/")
        || file.contains("/compose/")
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let facade_files: std::collections::BTreeSet<_> = index.records.iter()
        .filter(|r| !r.test && !operation(&r.file))
        .map(|r| &r.file)
        .filter(|file| index.records.iter().filter(|r| !r.test && &r.file == *file).all(|r| {
            matches!(&r.item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)))
                || matches!(&r.item, syn::Item::Mod(_))
        }))
        .collect();
    let http_files: std::collections::BTreeSet<_> = index
        .records
        .iter()
        .filter(|r| !r.test && (r.file.ends_with("/handlers.rs") || r.file.ends_with("/routes.rs")))
        .filter(|r| {
            r.references
                .iter()
                .any(|p| p.first().is_some_and(|s| s == "axum"))
        })
        .map(|r| &r.file)
        .collect();
    let http_modules: std::collections::BTreeSet<_> = index
        .records
        .iter()
        .filter(|r| http_files.contains(&r.file))
        .map(|r| r.module.as_str())
        .collect();
    let mut out = Vec::new();
    for r in index
        .records
        .iter()
        .filter(|r| !r.test && !adapter(&r.file) && !facade_files.contains(&r.file))
    {
        if annotations::permits(r, RULE) {
            continue;
        }
        let paths: std::collections::BTreeSet<_> = r
            .references
            .iter()
            .filter(|p| p.len() > 1 || r.imports.iter().any(|i| i.name.as_ref() == p.first()))
            .map(|p| index.resolve(r, p).unwrap_or_else(|| p.join("::")))
            .filter(|p| {
                (operation(&r.file) && transport(p))
                    || http_modules
                        .iter()
                        .any(|m| p == m || p.starts_with(&format!("{m}::")))
            })
            .collect();
        for path in paths {
            out.push(Violation::at(RULE, format!("{} -> {path}", r.id), "domain and persistence operations must not depend on HTTP handlers, routes or axum", r.location()));
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
    fn aliases_cannot_reverse_the_transport_dependency() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod service; mod handlers; mod error;");
        f.write(
            "sample/src/handlers.rs",
            "pub fn call() { crate::service::read(); }",
        );
        f.write(
            "sample/src/service.rs",
            "use crate::handlers::call as transport; pub fn read() { transport(); }",
        );
        f.write(
            "sample/src/error.rs",
            "impl axum::response::IntoResponse for Error {}",
        );
        let found = check(&f.load()).expect("check");
        assert!(!found.is_empty());
        assert!(
            found
                .iter()
                .all(|v| v.locations[0].contains("/service.rs:"))
        );
    }

    #[test]
    fn named_http_owners_are_distinct_from_domain_handler_values() {
        let mut f = Fixture::new();
        f.member(
            "sample",
            "",
            "mod service; mod handlers; mod mutation; mod authentication;",
        );
        f.write(
            "sample/src/handlers.rs",
            "use axum::Json; pub fn response() -> Json<()> { Json(()) }",
        );
        f.write(
            "sample/src/mutation.rs",
            "pub fn mutate() { crate::handlers::response(); }",
        );
        f.write("sample/src/service.rs", "pub fn run() { let handlers = Vec::<()>::new(); let routes = handlers; consume(routes); }");
        f.write(
            "sample/src/authentication.rs",
            "pub fn middleware() { crate::handlers::response(); }",
        );
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].key.contains("::mutate"));
    }

    #[test]
    fn transport_facades_do_not_hide_operation_dependencies() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod handlers; pub mod provider; mod service;");
        f.write("sample/src/handlers.rs", "use axum::Json; pub fn response() -> Json<()> { Json(()) }");
        f.write("sample/src/provider.rs", "pub use crate::handlers::response;");
        f.write("sample/src/service.rs", "pub fn run() { crate::provider::response(); }");
        let found = check(&f.load()).expect("check");
        assert!(!found.is_empty());
        assert!(found.iter().all(|v| v.locations[0].contains("/service.rs:")));
    }
}
