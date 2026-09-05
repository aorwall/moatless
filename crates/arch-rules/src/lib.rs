//! Architecture rules for the Moatless workspace.
//!
//! A rule reads the workspace and returns [`Violation`]s. `arch-report` prints
//! them and exits 0; `tests/architecture.rs` fails on ones absent from
//! `arch-baseline.json`, or on any of them under `ARCH_STRICT=1`.

pub mod baseline;
pub mod crate_deps;
pub mod duplicate_names;
pub mod module_names;
pub mod public_api;
pub mod workspace;

use anyhow::Result;
use workspace::Workspace;

/// One rule breach.
///
/// `key` identifies the breach across unrelated edits so a baseline entry does
/// not churn when a line moves. It never contains a line number; `locations`
/// carries those, and they are printed but not baselined.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: &'static str,
    pub key: String,
    pub message: String,
    pub locations: Vec<String>,
}

impl Violation {
    /// The stable identity a baseline entry records.
    pub fn id(&self) -> String {
        format!("{}::{}", self.rule, self.key)
    }
}

/// Every rule, in the order the report prints them.
pub fn run_all(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut found = Vec::new();
    found.extend(duplicate_names::check(ws)?);
    found.extend(crate_deps::check(ws));
    found.extend(module_names::check(ws)?);
    found.sort();
    Ok(found)
}

/// The rules that exist, so the report can name one that found nothing.
pub const RULES: &[(&str, &str)] = &[
    (
        duplicate_names::RULE,
        "A public type name is defined in more than one shared-vocabulary crate.",
    ),
    (
        crate_deps::RULE,
        "A crate names a dependency its band forbids.",
    ),
    (
        module_names::RULE,
        "A module is named for the kind of thing it holds rather than for its domain.",
    ),
];
