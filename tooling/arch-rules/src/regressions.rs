//! Boundary fixtures for the migrated checks.

use crate::fixtures::Fixture;

#[test]
fn band_edges_include_aliases_optional_and_target_dependencies() {
    for declaration in [
        "higher = { package = 'moatless-loops', path = '../moatless-loops', optional = true }",
        "[target.'cfg(unix)'.dependencies]\nhigher = { package = 'moatless-loops', path = '../moatless-loops' }",
    ] {
        let mut f = Fixture::new();
        f.member("moatless-loops", "", "");
        f.member("moatless-sandbox-k8s", declaration, "");
        let found = crate::band_order::check(&f.load()).expect("band order");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "moatless-sandbox-k8s -> moatless-loops");
    }
}

#[test]
fn dependency_exceptions_cover_exactly_one_edge() {
    let mut f = Fixture::new();
    f.member("moatless-loops", "", "");
    f.member("moatless-sandbox-control", "", "");
    f.member("moatless-sandbox-k8s", "moatless-loops = { path = '../moatless-loops' }\nmoatless-sandbox-control = { path = '../moatless-sandbox-control' }\n[[package.metadata.arch.allow]]\nrule = 'band-order'\ndependency = 'moatless-sandbox-control'\nreason = 'Watcher invokes lifecycle transitions'", "");
    let ws = f.load();
    assert!(
        crate::annotations::check(&ws)
            .expect("annotations")
            .is_empty()
    );
    let found = crate::band_order::check(&ws).expect("band order");
    assert_eq!(found.len(), 1);
    assert!(found[0].key.ends_with("moatless-loops"));
}

#[test]
fn dev_and_build_edges_do_not_change_production_bands() {
    let mut f = Fixture::new();
    f.member("moatless-loops", "", "");
    f.member("moatless-types", "[dev-dependencies]\nmoatless-loops = { path = '../moatless-loops' }\n[build-dependencies]\nmoatless-loops = { path = '../moatless-loops' }", "");
    assert!(
        crate::band_order::check(&f.load())
            .expect("band order")
            .is_empty()
    );
}

#[test]
fn aliases_cannot_hide_dao_construction_but_methods_are_not_constructors() {
    for (body, fails) in [
        ("Store::new()", true),
        ("Store()", true),
        ("Store::read()", false),
        ("<Store>::new()", true),
    ] {
        let mut f = Fixture::new();
        f.member("moatless-tasks", "", &format!("pub mod dao {{ pub struct PgTaskDao; }} use dao::PgTaskDao as Store; fn call() {{ {body}; }}"));
        assert_eq!(
            !crate::composition_root::check(&f.load())
                .expect("composition")
                .is_empty(),
            fails
        );
    }
}

#[test]
fn a_routing_file_with_a_handler_loses_the_state_exemption() {
    for (extra, fails) in [("", false), ("async fn handler() {}", true)] {
        let mut f = Fixture::new();
        f.member("moatless-backend", "", "mod routes;");
        f.write(
            "moatless-backend/src/routes.rs",
            &format!("fn routes(state: AppState) {{}} {extra}"),
        );
        assert_eq!(
            !crate::composition_root::check(&f.load())
                .expect("composition")
                .is_empty(),
            fails
        );
    }
}

#[test]
fn bindings_cannot_forward_or_alias_the_adapter_aggregate() {
    let mut f = Fixture::new();
    f.member("moatless-loops", "", "pub mod adapters { pub struct Aggregate; } pub mod bindings { use crate::adapters::Aggregate as Hidden; fn run(_: Hidden) {} }");
    assert!(
        !crate::adapter_boundaries::check(&f.load())
            .expect("boundaries")
            .is_empty()
    );
}

#[test]
fn generated_value_enums_are_indexed() {
    let mut f = Fixture::new();
    f.member(
        "moatless-types",
        "",
        "string_enum! { Status { Ready => \"ready\" } }",
    );
    let ws = f.load();
    assert!(
        ws.index()
            .expect("index")
            .records
            .iter()
            .any(|r| r.id == "moatless_types::Status" && r.exported)
    );
}

#[test]
fn wildcard_reexports_count_as_consumers() {
    let mut f = Fixture::new();
    f.member("moatless-types", "", "pub mod value { pub struct Shared; }");
    f.member(
        "moatless-api-schemas",
        "moatless-types = { path = '../moatless-types' }",
        "pub use moatless_types::value::*;",
    );
    f.member(
        "moatless-tasks",
        "moatless-types = { path = '../moatless-types' }",
        "fn use_it(_: moatless_types::value::Shared) {}",
    );
    assert!(
        crate::floor_type_admission::check(&f.load())
            .expect("admission")
            .is_empty()
    );
}

#[test]
fn a_generated_mock_definition_needs_an_exercised_constructor() {
    for used in [false, true] {
        let mut f = Fixture::new();
        let call = if used {
            "let fake = MockFake::new();"
        } else {
            ""
        };
        f.member("sample", "", &format!("pub trait Reader {{}} struct Store; impl Reader for Store {{}} #[cfg(test)] mod tests {{ use super::*; mock! {{ pub Fake {{}} impl Reader for Fake {{}} }} #[test] fn reads() {{ {call} }} }}"));
        assert_eq!(
            crate::single_implementation_trait::check(&f.load())
                .expect("trait rule")
                .len(),
            usize::from(!used)
        );
    }
}
