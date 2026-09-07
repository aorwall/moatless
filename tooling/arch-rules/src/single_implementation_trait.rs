//! A public trait needs another implementation, or an exercised test double.

use crate::source_index::implemented_trait;
use crate::source_index::{Index, Record};
use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;

pub const RULE: &str = "single-implementation-trait";

fn is_test_entry(r: &Record) -> bool {
    if let syn::Item::Fn(f) = &r.item {
        return f.attrs.iter().any(|a| {
            a.path()
                .segments
                .last()
                .is_some_and(|s| matches!(s.ident.to_string().as_str(), "test" | "rstest" | "case"))
        });
    }
    false
}

fn reachable_tests(index: &Index) -> BTreeSet<String> {
    let graph: BTreeMap<String, BTreeSet<String>> = index
        .records
        .iter()
        .filter(|r| matches!(r.item, syn::Item::Fn(_)))
        .map(|r| {
            let mut used = crate::source_index::ReferenceCollector {
                include_tests: true,
                omit_import_paths: true,
                ..Default::default()
            };
            used.visit_item(&r.item);
            (
                r.id.clone(),
                used.paths
                    .iter()
                    .filter_map(|p| index.resolve(r, p))
                    .collect(),
            )
        })
        .collect();
    let mut reachable: BTreeSet<String> = index
        .records
        .iter()
        .filter(|r| is_test_entry(r))
        .map(|r| r.id.clone())
        .collect();
    let mut pending: Vec<String> = reachable.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        for path in graph.get(&id).into_iter().flatten() {
            if reachable.insert(path.clone()) {
                pending.push(path.clone());
            }
        }
    }
    reachable
}

fn exercised(reachable: &BTreeSet<String>, id: &str) -> bool {
    reachable
        .iter()
        .any(|p| p == id || p.starts_with(&format!("{id}::")))
}

fn implementation_type(index: &Index, r: &Record) -> Option<String> {
    let syn::Item::Impl(i) = &r.item else {
        return None;
    };
    let syn::Type::Path(p) = &*i.self_ty else {
        return None;
    };
    index.resolve(
        r,
        &p.path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>(),
    )
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let reachable = reachable_tests(index);
    let mut out = Vec::new();
    for r in &index.records {
        let syn::Item::Trait(t) = &r.item else {
            continue;
        };
        if r.test || !matches!(t.vis, syn::Visibility::Public(_)) || annotations::permits(r, RULE) {
            continue;
        }
        let mut production = BTreeSet::new();
        let mut mocks = BTreeSet::new();
        for i in &index.records {
            if implemented_trait(index, i).as_ref() != Some(&r.id) {
                continue;
            }
            if let Some(ty) = implementation_type(index, i) {
                if i.test {
                    if exercised(&reachable, &ty) {
                        mocks.insert(ty);
                    }
                } else {
                    production.insert(ty);
                }
            }
        }
        let automock = t.attrs.iter().any(|a| match &a.meta {
            syn::Meta::List(l) => l.tokens.to_string().contains("automock"),
            _ => a
                .path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "automock"),
        });
        let mock = format!("{}::Mock{}", r.module, t.ident);
        if automock && exercised(&reachable, &mock) {
            mocks.insert(mock);
        }
        if production.len() < 2 && (production.is_empty() || mocks.is_empty()) {
            out.push(Violation::at(
                RULE,
                format!(
                    "{} implemented by {}",
                    r.id,
                    production.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
                format!(
                    "{} production implementation(s), {} exercised test double(s)",
                    production.len(),
                    mocks.len()
                ),
                &r.file,
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_spelling_in_another_module_is_not_mock_use() {
        let reached = BTreeSet::from(["other::MockReader::new".to_string()]);
        assert!(!exercised(&reached, "ports::MockReader"));
        assert!(exercised(&reached, "other::MockReader"));
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rule must run")]
mod fixtures {
    use super::*;
    use crate::fixtures::Fixture;
    #[test]
    fn a_generated_mock_must_be_used_from_a_test() {
        for used in [false, true] {
            let mut f = Fixture::new();
            let use_mock = if used {
                "let _reader = MockReader::new();"
            } else {
                "use super::MockReader;"
            };
            f.member("sample", "", &format!("#[cfg_attr(test, mockall::automock)] pub trait Reader {{}} struct Store; impl Reader for Store {{}} #[cfg(test)] mod tests {{ use super::*; #[test] fn reads() {{ {use_mock} }} }}"));
            assert_eq!(check(&f.load()).expect("rule").len(), usize::from(!used));
        }
    }
    #[test]
    fn a_handwritten_mock_in_an_unused_helper_does_not_qualify() {
        for used in [false, true] {
            let mut f = Fixture::new();
            let call = if used { "helper();" } else { "" };
            f.member("sample", "", &format!("pub trait Reader {{}} struct Store; impl Reader for Store {{}} #[cfg(test)] mod tests {{ use super::*; struct Fake; impl Reader for Fake {{}} fn helper() {{ let _reader = Fake; }} #[test] fn reads() {{ {call} }} }}"));
            assert_eq!(check(&f.load()).expect("rule").len(), usize::from(!used));
        }
    }
}
