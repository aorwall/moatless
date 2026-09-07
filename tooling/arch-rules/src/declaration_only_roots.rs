//! Library and namespace roots contain declarations and re-exports.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::source_index::Record;
use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "declaration-only-roots";

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.role_index()?;
    let executables: BTreeSet<_> = ws
        .source_members()
        .flat_map(|m| &m.targets)
        .filter(|t| t.executable())
        .map(|t| ws.relative(&t.path))
        .collect();
    let libraries: BTreeSet<_> = ws
        .source_members()
        .flat_map(|m| &m.targets)
        .filter(|t| t.library())
        .map(|t| ws.relative(&t.path))
        .collect();
    let namespaces: BTreeSet<_> = index
        .records
        .iter()
        .filter(|r| !r.test)
        .filter(|r| matches!(r.item, syn::Item::Mod(_)))
        .flat_map(|r| {
            let mut modules = vec![(r.file.clone(), r.module.clone())];
            if let syn::Item::Mod(m) = &r.item {
                if m.content.is_some() {
                    modules.push((r.file.clone(), format!("{}::{}", r.module, m.ident)));
                }
            }
            modules
        })
        .collect();
    let mut out = Vec::new();
    for r in &index.records {
        if r.test || executables.contains(&r.file) {
            continue;
        }
        let root = libraries.contains(&r.file)
            || r.file.ends_with("/mod.rs")
            || namespaces.contains(&(r.file.clone(), r.module.clone()));
        if !root || annotations::permits(r, RULE) || declaration(r) {
            continue;
        }
        out.push(Violation::at(
            RULE,
            &r.id,
            "root contains implementation; move it to its named owner",
            r.location(),
        ));
    }
    Ok(out)
}

fn declaration(r: &Record) -> bool {
    match &r.item {
        syn::Item::Use(_) | syn::Item::ExternCrate(_) => true,
        syn::Item::Mod(_) => true,
        syn::Item::Macro(m) => generated_macro(&m.mac),
        _ => false,
    }
}

fn generated_macro(m: &syn::Macro) -> bool {
    let path = m
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::");
    path == "tonic::include_proto"
        || (path == "include" && m.tokens.to_string().contains("OUT_DIR"))
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;

    #[test]
    fn declarations_generated_wrappers_and_test_children_are_distinct() {
        let mut f = Fixture::new();
        f.member("sample", "", "mod leaf; mod namespace; pub use leaf::Value; pub mod generated { tonic::include_proto!(\"wire\"); }");
        f.write(
            "sample/src/leaf.rs",
            "pub struct Value; #[cfg(test)] mod tests;",
        );
        f.write("sample/src/leaf/tests.rs", "#[test] fn works() {}");
        f.write(
            "sample/src/namespace.rs",
            "mod child; pub struct Misplaced;",
        );
        f.write("sample/src/namespace/child.rs", "pub struct Child;");
        let found = check(&f.load()).expect("check");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].key.ends_with("::Misplaced"));
    }

    #[test]
    fn custom_library_roots_and_inline_implementation_are_checked() {
        let mut f = Fixture::new();
        f.member("sample", "", "");
        f.write(
            "sample/Cargo.toml",
            "[package]\nname='sample'\nversion='0.1.0'\n[lib]\npath='custom/root.rs'\n",
        );
        f.write(
            "sample/custom/root.rs",
            "mod child; pub struct Wrong; mod inline { pub fn wrong() {} } opaque!();",
        );
        f.write("sample/custom/child.rs", "pub struct Fine;");
        let found = check(&f.load()).expect("check");
        assert!(found.iter().any(|v| v.key.ends_with("::Wrong")));
        assert!(found.iter().any(|v| v.key.ends_with("::inline::wrong")));
        assert!(found.iter().any(|v| v.key.ends_with("macro opaque")));
    }
}
