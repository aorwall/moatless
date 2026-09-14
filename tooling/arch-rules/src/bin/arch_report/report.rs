use std::collections::BTreeMap;

use anyhow::Result;
use moatless_arch_rules::{
    RULES, baseline, clippy_fence, persistence_export, run_all, workspace::Workspace,
    workspace::workspace_root,
};

pub(super) fn run(update: bool, json: bool, inventory: bool) -> Result<()> {
    let root = workspace_root();
    let ws = Workspace::load(&root)?;

    if inventory {
        // The inventory stays in `.plans/` while its plan sits in `.plans/completed/`.
        // Move it and this write lands somewhere nothing reads.
        let path = root.join(".plans/dao-persistence-migration-inventory.md");
        std::fs::write(&path, persistence_export::inventory(&ws)?)?;
        println!("Wrote {}", path.display());
        return Ok(());
    }

    let found = run_all(&ws)?;

    if update {
        baseline::write(&root, &found)?;
        println!(
            "Wrote {} violation(s) to {}",
            found.len(),
            baseline::path(&root).display()
        );
        return Ok(());
    }

    if json {
        let values: Vec<_> = found.iter().map(|v| serde_json::json!({"id":v.id(), "rule":v.rule, "key":v.key, "message":v.message, "locations":v.locations})).collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    let known = baseline::load(&root)?;
    let delta = baseline::diff(&known, &found);

    let mut by_rule: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for violation in &found {
        by_rule.entry(violation.rule).or_default().push(violation);
    }

    println!("Architecture report\n");

    for rule in RULES {
        let hits = by_rule.get(rule.name).map_or(&[][..], Vec::as_slice);
        println!("── {} ── {} violation(s)", rule.name, hits.len());
        println!("   {}\n", rule.about);
        for violation in hits {
            let marker = if known.contains(&violation.id()) {
                "known"
            } else {
                " NEW "
            };
            println!("   [{marker}] {}: {}", violation.key, violation.message);
            for location in &violation.locations {
                println!("           {location}");
            }
        }
        if !hits.is_empty() {
            println!();
        }
    }

    // Printed in full rather than only where a fence is broken: an entry this
    // rule cannot judge is the one thing a reader must not mistake for a pass.
    println!("── clippy fence ──");
    for entry in clippy_fence::entries(&ws)? {
        let state = match entry.state {
            clippy_fence::State::Resolved => "resolves",
            clippy_fence::State::Skipped => "skipped — names no workspace crate",
            clippy_fence::State::Unresolved => "UNRESOLVED",
        };
        println!("   {} — {state}", entry.path);
        // Only where the state is not a pass: a line per entry would bury the
        // skip this section exists to make visible once the fence list grows.
        if entry.state != clippy_fence::State::Resolved {
            println!("           {}", entry.location);
        }
    }
    println!();

    println!("── summary ──");
    println!("   {} violation(s) total", found.len());
    println!("   {} in the baseline", known.len());
    println!("   {} new", delta.new.len());
    if !delta.stale.is_empty() {
        println!(
            "   {} baseline entry(s) now fixed — run with --update-baseline to drop them",
            delta.stale.len()
        );
    }
    if found.is_empty() {
        println!("\n   Nothing left. Set ARCH_STRICT=1 in CI to keep it that way.");
    }
    Ok(())
}
