// A failed rule read is a failed test either way; the panic carries the
// message, as in `crates/server/tests/`.
#![allow(clippy::expect_used)]

//! The enforcing half.
//!
//! Each rule is one test, so a failure names the rule in the test list rather
//! than in a wall of shell output. Every test reads one `cached_violations`
//! run; `AGENTS.md` owns why that requires `cargo test` and not nextest. A test
//! fails on a violation absent from `arch-baseline.json`; with `ARCH_STRICT=1`
//! it fails on any violation at all.

use std::collections::BTreeSet;

use moatless_arch_rules::workspace::{Workspace, workspace_root};
use moatless_arch_rules::{
    Violation, agents_md_budget, baseline, dependency_privacy, duplicate_comment_prose,
    duplicate_constant_value, duplicate_names, module_names, vendor_crate_owner,
    vendor_type_export,
};

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

/// The one workspace read every rule test shares.
fn cached_violations() -> Vec<Violation> {
    static FOUND: std::sync::OnceLock<Vec<Violation>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let ws = Workspace::load(workspace_root()).expect("loading the workspace");
            moatless_arch_rules::run_all(&ws).expect("running architecture rules")
        })
        .clone()
}

#[test]
fn one_public_type_name_means_one_thing() {
    assert_rule(duplicate_names::RULE, cached_violations());
}

#[test]
fn no_crate_names_a_dependency_its_band_forbids() {
    assert_rule(dependency_privacy::RULE, cached_violations());
}

#[test]
fn a_module_is_named_for_its_domain() {
    assert_rule(module_names::RULE, cached_violations());
}

#[test]
fn an_agents_md_stays_within_its_line_budget() {
    assert_rule(agents_md_budget::RULE, cached_violations());
}

#[test]
fn comment_prose_is_not_copied_between_files() {
    assert_rule(duplicate_comment_prose::RULE, cached_violations());
}

#[test]
fn a_literal_constant_value_has_one_owning_crate() {
    assert_rule(duplicate_constant_value::RULE, cached_violations());
}

/// A baseline entry that no longer occurs must be removed, or the file stops
/// shrinking and the next violation inherits an accepted slot.
#[test]
fn the_baseline_holds_no_fixed_violations() {
    let root = workspace_root();
    let found = cached_violations();
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

#[test]
fn every_registered_rule_is_enforced() {
    let found = cached_violations();
    let rules: BTreeSet<_> = moatless_arch_rules::RULES.iter().map(|r| r.name).collect();
    assert_eq!(rules.len(), moatless_arch_rules::RULES.len());
    assert!(found.iter().all(|v| rules.contains(v.rule)));
    for rule in rules {
        assert_rule(rule, found.clone());
    }
}

macro_rules! enforce {
    ($($rule:ident),+ $(,)?) => {$(
        #[test]
        fn $rule() {
            assert_rule(moatless_arch_rules::$rule::RULE, cached_violations());
        }
    )+};
}

enforce!(
    declaration_only_roots,
    route_registration,
    transport_direction,
    persistence_sql_location,
    persistence_export,
    process_exit,
    env_read_owner,
    reqwest_default,
    annotations,
    clippy_fence,
    row_stays_with_query,
    agents_symlink,
    floor_type_admission,
    port_admission,
    single_implementation_trait,
    composition_root,
    table_ownership,
    store_privacy,
    adapter_boundaries,
    band_order,
    manifest_inheritance,
    closed_dependency_list,
    base_surface,
    file_size,
    conversation_writer,
    session_line_parsing,
    sqlx_test_migrations,
    migration_contract
);

#[test]
fn a_vendor_sdk_is_declared_only_by_its_owner() {
    assert_rule(vendor_crate_owner::RULE, cached_violations());
}

#[test]
fn no_public_api_names_a_vendor_sdk_type() {
    assert_rule(vendor_type_export::RULE, cached_violations());
}
