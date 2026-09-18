//! Adapter module boundaries which cannot be expressed as Cargo edges.

use crate::workspace::Workspace;
use crate::{Violation, annotations};
use anyhow::Result;

pub const RULE: &str = "adapter-boundaries";
pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let index = ws.index()?;
    let mut out = Vec::new();
    for r in &index.records {
        if r.test || r.package != "moatless-loops" || annotations::permits(r, RULE) {
            continue;
        }
        if r.module.starts_with("moatless_loops::bindings")
            && index
                .paths(r)
                .any(|p| p.starts_with("moatless_loops::adapters::"))
        {
            out.push(Violation::at(
                RULE,
                format!("{} -> adapters", r.id),
                "Binding code may not name the Adapter aggregate",
                &r.file,
            ));
        }
        if r.module.starts_with("moatless_loops::adapters") {
            if let syn::Item::Use(u) = &r.item {
                if matches!(u.vis, syn::Visibility::Public(_))
                    && index
                        .paths(r)
                        .any(|p| p.starts_with("moatless_") && !p.starts_with("moatless_loops::"))
                {
                    out.push(Violation::at(
                        RULE,
                        format!("{} forwards a foreign item", r.id),
                        "name Adapter vocabulary directly from its owner",
                        &r.file,
                    ));
                }
            }
        }
    }
    // This legacy restriction includes documentation and tests, not only compiled paths.
    for name in [
        "moatless-adapter-slack",
        "moatless-adapter-webhook",
        "moatless-adapter-telegram",
    ] {
        if let Some(m) = ws.member(name) {
            for (file, text) in ws.sources(m)? {
                if text.contains("moatless_loops")
                    && !(name == "moatless-adapter-telegram"
                        && file.ends_with("/src/pairing/approval.rs"))
                {
                    out.push(Violation::at(
                        RULE,
                        format!("{file} names moatless_loops"),
                        "this provider may not name the loops crate",
                        file,
                    ));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
