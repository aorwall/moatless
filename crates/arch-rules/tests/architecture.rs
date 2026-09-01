// A failed rule read is a failed test either way; the panic carries the
// message, as in `crates/backend/tests/`.
#![allow(clippy::expect_used)]

//! The enforcing half.
//!
//! Each rule is one test, so a failure names the rule in the test list rather
//! than in a wall of shell output. A test fails on a violation that is not in
//! `arch-baseline.json`; with `ARCH_STRICT=1` it fails on any violation at
//! all, which is the switch to flip once a rule's baseline reaches zero.

use std::collections::BTreeSet;

use moatless_arch_rules::workspace::{workspace_root, Workspace};
use moatless_arch_rules::{baseline, crate_deps, duplicate_names, module_names, Violation};

fn strict() -> bool {
    std::env::var("ARCH_STRICT").is_ok_and(|v| v == "1")
}

/// Assert one rule's violations are all accounted for.
///
/// Strict mode reads the baseline as empty rather than taking its own branch,
/// so both modes report through one path and one message.
fn assert_rule(rule: &str, found: Vec<Violation>) {
    let root = workspace_root();
    let known = if strict() {
        BTreeSet::new()
    } else {
        baseline::load(&root).expect("reading the baseline")
    };

    let unlisted: Vec<Violation> = found
        .into_iter()
        .filter(|v| v.rule == rule && !known.contains(&v.id()))
        .collect();
    let hint = if strict() {
        "ARCH_STRICT is set, so the baseline does not excuse them."
    } else {
        "Fix them, or record them deliberately with:\n  \
         cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline"
    };
    assert!(
        unlisted.is_empty(),
        "{} unaccounted violation(s) of `{rule}`:\n{}\n{hint}",
        unlisted.len(),
        render(&unlisted)
    );
}

fn render(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|v| format!("  - {}\n      {}", v.message, v.locations.join("\n      ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn workspace() -> Workspace {
    Workspace::load(workspace_root()).expect("loading the workspace")
}

#[test]
fn one_public_type_name_means_one_thing() {
    let ws = workspace();
    let found = duplicate_names::check(&ws).expect("running the duplicate-name rule");
    assert_rule(duplicate_names::RULE, found);
}

#[test]
fn no_crate_names_a_dependency_its_band_forbids() {
    let ws = workspace();
    assert_rule(crate_deps::RULE, crate_deps::check(&ws));
}

#[test]
fn a_module_is_named_for_its_domain() {
    let ws = workspace();
    let found = module_names::check(&ws).expect("running the module-name rule");
    assert_rule(module_names::RULE, found);
}

/// A baseline entry that no longer occurs must be removed, or the file stops
/// shrinking and the next violation inherits an accepted slot.
#[test]
fn the_baseline_holds_no_fixed_violations() {
    let root = workspace_root();
    let ws = workspace();
    let found = moatless_arch_rules::run_all(&ws).expect("running every rule");
    let known = baseline::load(&root).expect("reading the baseline");
    let stale = baseline::diff(&known, &found).stale;
    assert!(
        stale.is_empty(),
        "{} baseline entry(s) are fixed and must be dropped:\n{}\n\
         Run: cargo run -p moatless-arch-rules --bin arch-report -- --update-baseline",
        stale.len(),
        stale
            .iter()
            .map(|s| format!("  - {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
