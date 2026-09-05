//! Architecture rules for the Moatless workspace.
//!
//! A rule reads the workspace and returns [`Violation`]s. `arch-report` prints
//! them and exits 0; `tests/architecture.rs` fails on ones absent from
//! `arch-baseline.json`, or on any of them under `ARCH_STRICT=1`.

pub mod adapter_boundaries;
pub mod agents_symlink;
pub mod annotations;
pub mod band_order;
pub mod base_surface;
pub mod baseline;
pub mod closed_dependency_list;
pub mod composition_root;
pub mod dependency_privacy;
pub mod duplicate_names;
pub mod floor_type_admission;
pub mod manifest_inheritance;
pub mod module_names;
pub mod port_admission;
pub mod public_api;
pub mod row_stays_with_query;
pub mod single_implementation_trait;
pub mod source_index;
pub mod sql;
pub mod store_privacy;
pub mod table_ownership;
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
    /// A breach identified by its participants and located in the source tree.
    pub fn at(
        rule: &'static str,
        key: impl Into<String>,
        message: impl Into<String>,
        file: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            key: key.into(),
            message: message.into(),
            locations: vec![file.into()],
        }
    }

    /// The stable identity a baseline entry records.
    pub fn id(&self) -> String {
        format!("{}::{}", self.rule, self.key)
    }
}

/// Every rule, in the order the report prints them.
pub fn run_all(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut found = Vec::new();
    found.extend(duplicate_names::check(ws)?);
    found.extend(dependency_privacy::check(ws));
    found.extend(module_names::check(ws)?);
    found.extend(annotations::check(ws)?);
    found.extend(band_order::check(ws));
    found.extend(manifest_inheritance::check(ws));
    found.extend(closed_dependency_list::check(ws));
    found.extend(base_surface::check(ws));
    found.extend(row_stays_with_query::check(ws)?);
    found.extend(agents_symlink::check(ws)?);
    found.extend(floor_type_admission::check(ws)?);
    found.extend(port_admission::check(ws)?);
    found.extend(single_implementation_trait::check(ws)?);
    found.extend(composition_root::check(ws)?);
    found.extend(table_ownership::check(ws)?);
    found.extend(store_privacy::check(ws)?);
    found.extend(adapter_boundaries::check(ws)?);
    found.sort();
    Ok(found)
}

/// The rules that exist, so the report can name one that found nothing.
pub const RULES: &[(&str, &str)] = &[
    (
        adapter_boundaries::RULE,
        "Bindings and providers keep their Adapter module boundaries.",
    ),
    (
        store_privacy::RULE,
        "Store types and drivers stay with table owners.",
    ),
    (
        table_ownership::RULE,
        "Production SQL names only tables its crate owns.",
    ),
    (
        composition_root::RULE,
        "State and concrete stores are assembled at the composition root.",
    ),
    (
        single_implementation_trait::RULE,
        "A public trait has multiple implementations or an exercised mock.",
    ),
    (
        port_admission::RULE,
        "Ports contain contracts with independent consumers.",
    ),
    (
        floor_type_admission::RULE,
        "Shared values have independent consumers.",
    ),
    (
        agents_symlink::RULE,
        "Repository instructions have compatibility symlinks.",
    ),
    (
        row_stays_with_query::RULE,
        "SQL decoding stays with a driver-owning crate.",
    ),
    (
        base_surface::RULE,
        "Adapter transport dependencies are feature gated.",
    ),
    (
        closed_dependency_list::RULE,
        "Shared values declare only value-level dependencies.",
    ),
    (
        annotations::RULE,
        "Exceptions name a registered, applicable rule and carry a reason.",
    ),
    (
        band_order::RULE,
        "Workspace dependencies point strictly down the band order.",
    ),
    (
        manifest_inheritance::RULE,
        "Members inherit package identity and lints.",
    ),
    (
        duplicate_names::RULE,
        "A public type name is defined in more than one shared-vocabulary crate.",
    ),
    (
        dependency_privacy::RULE,
        "A crate names a dependency its band forbids.",
    ),
    (
        module_names::RULE,
        "A module is named for the kind of thing it holds rather than for its domain.",
    ),
];

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture I/O must succeed")]
pub(crate) mod fixtures;

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture rules must run")]
mod regressions;
