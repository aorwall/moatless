use crate::workspace::Workspace;
use crate::{
    Violation, adapter_boundaries, agents_symlink, annotations, band_order, base_surface,
    closed_dependency_list, composition_root, declaration_only_roots, dependency_privacy,
    duplicate_names, file_size, floor_type_admission, manifest_inheritance, module_names,
    persistence_export, persistence_sql_location, port_admission, process_exit, route_registration,
    row_stays_with_query, single_implementation_trait, store_privacy, table_ownership,
    transport_direction,
};
use anyhow::{Context, Result};

/// One rule: its name, what it asserts, and the check that finds its breaches.
pub struct Rule {
    pub name: &'static str,
    pub about: &'static str,
    pub check: fn(&Workspace) -> Result<Vec<Violation>>,
}

/// Register each rule from its own module, so a rule's name cannot be paired
/// with another rule's check.
macro_rules! rules {
    ($($module:ident => $about:literal),+ $(,)?) => {
        /// Every rule, in the order the report prints them.
        ///
        /// This is the single registration. [`run_all`] iterates it, so a rule
        /// cannot be named here and left out of the run.
        pub const RULES: &[Rule] = &[$(Rule {
            name: $module::RULE,
            about: $about,
            check: $module::check,
        }),+];
    };
}

rules!(
    declaration_only_roots => "Library and namespace roots contain declarations and re-exports.",
    route_registration => "Routes register handlers and middleware without operation bodies.",
    transport_direction => "Domain operations do not depend on HTTP adapters.",
    persistence_sql_location => "SQL execution stays with persistence owners.",
    persistence_export => "Domain persistence stays inside the crate that owns its tables.",
    process_exit => "Process termination stays in executable entry functions.",
    adapter_boundaries => "Bindings and providers keep their Adapter module boundaries.",
    store_privacy => "Store types and drivers stay with table owners.",
    table_ownership => "Production SQL names only tables its crate owns.",
    composition_root => "State and concrete stores are assembled at the composition root.",
    single_implementation_trait => "A public trait has multiple implementations or an exercised mock.",
    port_admission => "Ports contain contracts with independent consumers.",
    floor_type_admission => "Shared values have independent consumers.",
    agents_symlink => "Repository instructions have compatibility symlinks.",
    row_stays_with_query => "SQL decoding stays with a driver-owning crate.",
    base_surface => "Adapter transport dependencies are feature gated.",
    closed_dependency_list => "Shared values declare only value-level dependencies.",
    annotations => "Exceptions name a registered, applicable rule and carry a reason.",
    band_order => "Workspace dependencies point strictly down the band order.",
    manifest_inheritance => "Members inherit package identity and lints.",
    duplicate_names => "A public type name is defined in more than one shared-vocabulary crate.",
    dependency_privacy => "A crate names a dependency its band forbids.",
    module_names => "A module is named for the kind of thing it holds rather than for its domain.",
    file_size => "A source file stays under the line limit.",
);

/// Run every registered rule against one workspace read.
///
/// `ws` caches its parsed source index, so the whole set costs one parse.
pub fn run_all(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut found = Vec::new();
    for rule in RULES {
        // Name the rule: every test reads this one run, so a bare error would
        // panic all of them with the same message.
        found.extend((rule.check)(ws).with_context(|| format!("rule `{}`", rule.name))?);
    }
    found.sort();
    Ok(found)
}
