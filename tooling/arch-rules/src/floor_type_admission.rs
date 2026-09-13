//! Shared values need independent production consumers.

use crate::workspace::Workspace;
use crate::{Violation, annotations, band_order};
use anyhow::Result;
use std::collections::BTreeSet;

pub const RULE: &str = "floor-type-admission";

fn admitted(consumers: &BTreeSet<String>) -> bool {
    consumers.len() >= 3
        || (consumers.len() == 2
            && consumers
                .iter()
                .filter_map(|c| band_order::band(c))
                .collect::<BTreeSet<_>>()
                .len()
                == 2)
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let mut out = Vec::new();
    for item in index.records.iter().filter(|r| {
        matches!(
            r.package.as_str(),
            "moatless-types" | "moatless-workspace-types"
        ) && !(r.module == "moatless_workspace_types::schemas"
            || r.module.starts_with("moatless_workspace_types::schemas::"))
            && r.exported
            && matches!(
                r.item,
                syn::Item::Struct(_) | syn::Item::Enum(_) | syn::Item::Type(_)
            )
    }) {
        let consumers: BTreeSet<String> = index
            .records
            .iter()
            .filter(|r| !r.test && r.package != item.package && index.names(r, &item.id))
            .map(|r| r.package.clone())
            .collect();
        let shared_with_schema = item.package == "moatless-workspace-types"
            && !consumers.is_empty()
            && index.records.iter().any(|r| {
                r.package == item.package
                    && r.module.starts_with("moatless_workspace_types::schemas::")
                    && r.exported
                    && !r.test
                    && index.names(r, &item.id)
            });
        if !admitted(&consumers) && !shared_with_schema && !annotations::permits(item, RULE) {
            out.push(Violation::at(
                RULE,
                format!(
                    "{} read by {}",
                    item.id,
                    consumers.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
                format!(
                    "{} has {} consumer crates; needs three, or two in different bands",
                    item.id,
                    consumers.len()
                ),
                &item.file,
            ));
        }
    }
    for member in ws.members().filter(|m| {
        matches!(
            m.name.as_str(),
            "moatless-types" | "moatless-workspace-types"
        )
    }) {
        let allowed: BTreeSet<String> = member
            .dependencies
            .iter()
            .filter(|d| d.kind.is_none())
            .map(|d| d.alias.replace('-', "_"))
            .collect();
        let foreign: BTreeSet<String> = ws
            .members()
            .map(|m| m.name.replace('-', "_"))
            .filter(|name| name != &member.name.replace('-', "_") && !allowed.contains(name))
            .collect();
        for item in index.records.iter().filter(|r| {
            r.package == member.name
                && !r.test
                && matches!(
                    r.item,
                    syn::Item::Struct(_)
                        | syn::Item::Enum(_)
                        | syn::Item::Type(_)
                        | syn::Item::Impl(_)
                )
        }) {
            for path in &item.references {
                if path.first().is_some_and(|name| foreign.contains(name)) {
                    let path = path.join("::");
                    out.push(Violation::at(
                        RULE,
                        format!("{} names {path}", item.id),
                        "shared values may name only their declared dependencies",
                        &item.file,
                    ));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn readers(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }
    #[test]
    fn two_readers_must_cross_a_band() {
        assert!(!admitted(&readers(&[])));
        assert!(!admitted(&readers(&["moatless-tasks"])));
        assert!(!admitted(&readers(&[
            "moatless-sandbox-k8s",
            "moatless-sandbox-docker"
        ])));
        assert!(admitted(&readers(&["moatless-tasks", "moatless-t3code"])));
        assert!(admitted(&readers(&[
            "moatless-adapter-slack",
            "moatless-adapter-webhook",
            "moatless-adapter-linear"
        ])));
    }
}
