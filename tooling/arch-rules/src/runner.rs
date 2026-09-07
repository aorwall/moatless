use crate::workspace::Workspace;
use crate::{
    Violation, adapter_boundaries, agents_symlink, annotations, band_order, base_surface,
    closed_dependency_list, composition_root, declaration_only_roots, dependency_privacy,
    duplicate_names, floor_type_admission, manifest_inheritance, module_names,
    persistence_sql_location, port_admission, process_exit, route_registration,
    row_stays_with_query, single_implementation_trait, store_privacy, table_ownership,
    transport_direction,
};
use anyhow::Result;

/// Every rule, in the order the report prints them.
pub fn run_all(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut found = Vec::new();
    found.extend(declaration_only_roots::check(ws)?);
    found.extend(route_registration::check(ws)?);
    found.extend(transport_direction::check(ws)?);
    found.extend(persistence_sql_location::check(ws)?);
    found.extend(process_exit::check(ws)?);
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
        declaration_only_roots::RULE,
        "Library and namespace roots contain declarations and re-exports.",
    ),
    (
        route_registration::RULE,
        "Routes register handlers and middleware without operation bodies.",
    ),
    (
        transport_direction::RULE,
        "Domain operations do not depend on HTTP adapters.",
    ),
    (
        persistence_sql_location::RULE,
        "SQL execution stays with persistence owners.",
    ),
    (
        process_exit::RULE,
        "Process termination stays in executable entry functions.",
    ),
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
