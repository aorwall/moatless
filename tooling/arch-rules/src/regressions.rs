//! Boundary fixtures for the migrated checks.

use crate::fixtures::Fixture;

#[test]
fn workspaces_cannot_depend_directly_on_api_schemas() {
    for declaration in [
        "moatless-identity = { path = '../moatless-identity' }",
        "schemas = { package = 'moatless-api-schemas', path = '../moatless-api-schemas', optional = true }",
        "[target.'cfg(unix)'.dependencies]\nschemas = { package = 'moatless-api-schemas', path = '../moatless-api-schemas' }",
    ] {
        let mut f = Fixture::new();
        f.member("moatless-api-schemas", "", "");
        f.member(
            "moatless-identity",
            "moatless-api-schemas = { path = '../moatless-api-schemas' }",
            "",
        );
        f.member("moatless-workspaces", declaration, "");
        let found = crate::dependency_privacy::check(&f.load()).expect("dependency privacy");
        if declaration.starts_with("moatless-identity") {
            assert!(found.is_empty());
        } else {
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].key, "moatless-workspaces -> moatless-api-schemas");
        }
    }
}

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
fn a_vendor_exception_covers_a_dev_only_edge() {
    let mut f = Fixture::new();
    f.member("aws-sdk-s3", "", "");
    f.member("moatless-sandbox", "[dev-dependencies]\naws-sdk-s3 = { path = '../aws-sdk-s3' }\n[[package.metadata.arch.allow]]\nrule = 'vendor-crate-owner'\ndependency = 'aws-sdk-s3'\nreason = 'Fixture exception'", "");
    f.member(
        "moatless-cli",
        "[dev-dependencies]\naws-sdk-s3 = { path = '../aws-sdk-s3' }",
        "",
    );
    let ws = f.load();
    assert!(
        crate::annotations::check(&ws)
            .expect("annotations")
            .is_empty()
    );
    let found = crate::vendor_crate_owner::check(&ws).expect("vendor crate owner");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, "moatless-cli -> aws-sdk-s3");
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

#[test]
fn workspace_contracts_cannot_add_runtime_dependencies() {
    for package in ["moatless-workspace-types", "moatless-workspace-ports"] {
        let mut f = Fixture::new();
        f.member("sqlx", "", "");
        f.member(
            package,
            "db = { package = 'sqlx', path = '../sqlx', optional = true }",
            "",
        );
        let found = crate::closed_dependency_list::check(&f.load()).expect("dependency rule");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, format!("{package} -> sqlx"));
    }
}

#[test]
fn workspace_schema_aliases_cannot_enter_values_ports_or_drivers() {
    for package in [
        "moatless-workspace-types",
        "moatless-workspace-ports",
        "moatless-sandbox-k8s",
        "moatless-sandbox-docker",
    ] {
        let mut f = Fixture::new();
        let schemas = "pub mod schemas { pub struct Request; } pub use schemas::Request as Alias;";
        if package == "moatless-workspace-types" {
            f.member(package, "", &format!("{schemas} pub mod schemas_archive {{ pub struct Value {{ pub request: crate::Alias }} }}"));
        } else {
            f.member("moatless-workspace-types", "", schemas);
            f.member(package, "values = { package = 'moatless-workspace-types', path = '../moatless-workspace-types' }", "use values::Alias as Hidden; pub struct Value { pub request: Hidden }");
        }
        let found = crate::dependency_privacy::check(&f.load()).expect("schema rule");
        assert!(
            found.iter().any(|v| v
                .key
                .contains("::Value names moatless_workspace_types::schemas::Request")),
            "{package}"
        );
    }
}

#[test]
fn workspace_port_support_is_admitted_but_unrelated_values_are_not() {
    let mut f = Fixture::new();
    f.member(
        "moatless-workspace-ports",
        "",
        "pub struct Reply; pub struct Unrelated; pub trait Reader { fn read(&self) -> Reply; }",
    );
    for package in ["one", "two"] {
        f.member(
            package,
            "moatless-workspace-ports = { path = '../moatless-workspace-ports' }",
            "fn read(_: &dyn moatless_workspace_ports::Reader) {}",
        );
    }
    let found = crate::port_admission::check(&f.load()).expect("port rule");
    assert_eq!(found.len(), 1);
    assert!(found[0].key.ends_with("::Unrelated"));
}

#[test]
fn schema_shared_values_still_need_an_external_consumer() {
    for consumed in [false, true] {
        let mut f = Fixture::new();
        f.member("moatless-workspace-types", "", "pub mod workspace { pub enum Kind { One } } pub mod schemas { pub mod workspace { pub struct Response { pub kind: crate::workspace::Kind } } }");
        if consumed {
            f.member(
                "moatless-workspaces",
                "moatless-workspace-types = { path = '../moatless-workspace-types' }",
                "fn read(_: moatless_workspace_types::workspace::Kind) {}",
            );
        }
        let found = crate::floor_type_admission::check(&f.load()).expect("value admission");
        assert_eq!(found.is_empty(), consumed);
    }
}

#[test]
fn a_vendor_type_in_an_exported_signature_is_caught_in_its_owner() {
    let mut f = Fixture::new();
    f.member("aws-sdk-s3", "", "pub mod error { pub struct SdkError; }");
    f.member(
        "moatless-s3",
        "aws-sdk-s3 = { path = '../aws-sdk-s3' }",
        "use aws_sdk_s3::error::SdkError;\n\
         pub enum S3Error { Put(Box<SdkError>) }\n\
         pub struct S3Storage { client: aws_sdk_s3::error::SdkError, pub region: String }\n\
         impl S3Storage { pub fn raw(&self) -> &SdkError { todo!() } fn inner(&self) -> SdkError { todo!() } }\n\
         struct Hidden;\n\
         impl Hidden { pub fn raw(&self) -> SdkError { todo!() } }\n\
         pub fn fine(key: &str) -> String { let _e: SdkError; todo!() }\n\
         pub use aws_sdk_s3::error::SdkError as Leaked;\n\
         pub trait Store { type Error; }\n\
         impl Store for S3Storage { type Error = SdkError; }\n\
         pub struct Clean;\n\
         impl Store for Clean { type Error = String; }\n\
         impl From<SdkError> for S3Storage { fn from(e: SdkError) -> Self { todo!() } }\n\
         impl Store for Hidden { type Error = SdkError; }",
    );
    let found: Vec<String> = crate::vendor_type_export::check(&f.load())
        .expect("vendor type export")
        .into_iter()
        .map(|v| v.key)
        .collect();
    assert_eq!(
        found,
        [
            "moatless-s3 moatless_s3::Leaked -> aws-sdk-s3",
            "moatless-s3 moatless_s3::S3Error -> aws-sdk-s3",
            "moatless-s3 moatless_s3::S3Storage as From -> aws-sdk-s3",
            "moatless-s3 moatless_s3::S3Storage as Store -> aws-sdk-s3",
            "moatless-s3 moatless_s3::S3Storage::raw -> aws-sdk-s3",
        ]
    );
}
