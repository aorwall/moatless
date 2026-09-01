// A report binary's whole output is stdout, the same reason `moatless-cli`
// allows these at its own root.
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! Print every architecture violation in the workspace.
//!
//! ```text
//! cargo run -p moatless-arch-rules --bin arch-report
//! cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline
//! ```
//!
//! Exits 0 whatever it finds. Enforcement is the test harness's job, so this
//! stays usable as a to-do list while the counts come down.

use std::collections::BTreeMap;

use anyhow::Result;
use moatless_arch_rules::{baseline, run_all, workspace::Workspace, workspace::workspace_root, RULES};

fn main() -> Result<()> {
    let update = std::env::args().any(|a| a == "--update-baseline");
    let root = workspace_root();
    let ws = Workspace::load(&root)?;
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

    let known = baseline::load(&root)?;
    let delta = baseline::diff(&known, &found);

    let mut by_rule: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for violation in &found {
        by_rule.entry(violation.rule).or_default().push(violation);
    }

    println!("Architecture report\n");

    for (rule, description) in RULES {
        let hits = by_rule.get(rule).map_or(&[][..], Vec::as_slice);
        println!("── {rule} ── {} violation(s)", hits.len());
        println!("   {description}\n");
        for violation in hits {
            let marker = if known.contains(&violation.id()) {
                "known"
            } else {
                " NEW "
            };
            println!("   [{marker}] {}", violation.message);
            for location in &violation.locations {
                println!("           {location}");
            }
        }
        if !hits.is_empty() {
            println!();
        }
    }

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
