//! Ports expose traits and the value types their signatures require.

use crate::source_index::{Index, Record};
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::BTreeSet;

pub const RULE: &str = "port-admission";

/// The canonical trait implemented by an impl declaration.
pub fn implemented_trait(index: &Index, r: &Record) -> Option<String> {
    let syn::Item::Impl(i) = &r.item else {
        return None;
    };
    let (_, path, _) = i.trait_.as_ref()?;
    index.resolve(
        r,
        &path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>(),
    )
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let ports: Vec<&Record> = index
        .records
        .iter()
        .filter(|r| matches!(r.package.as_str(), "moatless-ports") && r.exported)
        .collect();
    let mut supporting = BTreeSet::new();
    for r in &ports {
        if matches!(r.item, syn::Item::Trait(_)) {
            supporting.extend(index.paths(r).cloned());
        }
    }
    loop {
        let before = supporting.len();
        for r in &ports {
            if supporting.contains(&r.id)
                && matches!(
                    r.item,
                    syn::Item::Struct(_) | syn::Item::Enum(_) | syn::Item::Type(_)
                )
            {
                supporting.extend(index.paths(r).cloned());
            }
        }
        if before == supporting.len() {
            break;
        }
    }
    let mut out = Vec::new();
    for r in ports {
        if annotations::permits(r, RULE) {
            continue;
        }
        if matches!(r.item, syn::Item::Trait(_)) {
            let implementers: BTreeSet<&str> = index
                .records
                .iter()
                .filter(|i| !i.test && implemented_trait(index, i).as_ref() == Some(&r.id))
                .map(|i| i.package.as_str())
                .collect();
            let consumers: BTreeSet<&str> = index
                .records
                .iter()
                .filter(|i| {
                    !i.test
                        && i.package != r.package
                        && !implementers.contains(i.package.as_str())
                        && index.names(i, &r.id)
                })
                .map(|i| i.package.as_str())
                .collect();
            if consumers.len() < 2 {
                out.push(Violation::at(RULE, format!("{} read by {}", r.id, consumers.iter().copied().collect::<Vec<_>>().join(", ")),
                    format!("trait consumers: {consumers:?}; implementers: {implementers:?}; needs two non-implementing consumers"), &r.file));
            }
        } else if matches!(
            r.item,
            syn::Item::Struct(_)
                | syn::Item::Enum(_)
                | syn::Item::Type(_)
                | syn::Item::Fn(_)
                | syn::Item::Const(_)
                | syn::Item::Static(_)
        ) && !supporting.contains(&r.id)
        {
            out.push(Violation::at(
                RULE,
                &r.id,
                "export is neither a port nor a supporting contract type",
                &r.file,
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rule must run")]
mod tests {
    use super::*;
    use crate::fixtures::Fixture;
    #[test]
    fn contract_types_stay_but_unrelated_exports_fail() {
        let mut f = Fixture::new();
        f.member("moatless-ports", "", "pub struct Detail; pub struct Reply { pub detail: Detail } pub trait Reader { fn read(&self) -> Reply; } pub struct Unrelated;");
        for name in ["one", "two"] {
            f.member(
                name,
                "moatless-ports = { path = '../moatless-ports' }",
                "pub fn read(_: &dyn moatless_ports::Reader) {}",
            );
        }
        let violations = check(&f.load()).expect("rule");
        assert_eq!(violations.len(), 1);
        assert!(violations[0].key.ends_with("::Unrelated"));
    }
    #[test]
    fn an_implementer_does_not_count_as_a_consumer() {
        let mut f = Fixture::new();
        f.member("moatless-ports", "", "pub trait Reader {}");
        f.member(
            "implementation",
            "moatless-ports = { path = '../moatless-ports' }",
            "struct Store; impl moatless_ports::Reader for Store {}",
        );
        f.member(
            "consumer",
            "moatless-ports = { path = '../moatless-ports' }",
            "pub fn read(_: &dyn moatless_ports::Reader) {}",
        );
        assert_eq!(check(&f.load()).expect("rule").len(), 1);
    }
}
