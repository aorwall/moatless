//! Source classifiers agree across target and export analysis.

use crate::fixtures::Fixture;
use crate::{declaration_only_roots, public_api, source_index};

#[test]
fn test_cfg_semantics_match_both_walkers() {
    let cases = [
        ("test", true),
        ("not(test)", false),
        ("any(test, feature = \"production\")", false),
        ("all(test, feature = \"production\")", true),
        ("feature = \"test-support\"", true),
        ("feature = \"db-tests\"", true),
    ];
    for (cfg, expected_test) in cases {
        let mut f = Fixture::new();
        f.member("sample", "", &format!("#[cfg({cfg})] pub struct Value;"));
        let ws = f.load();
        let index = ws.index().expect("index");
        let record = index
            .records
            .iter()
            .find(|r| r.id.ends_with("::Value"))
            .expect("value");
        assert_eq!(record.test, expected_test, "{cfg}");
        assert_eq!(
            source_index::test_only(source_index::attrs(&record.item)),
            expected_test,
            "{cfg}"
        );
        let member = ws.member("sample").expect("member");
        let sources = ws.sources(member).expect("sources").into_iter().collect();
        let exports =
            public_api::exports(&sources, &ws.relative(&member.targets[0].path)).expect("exports");
        assert_eq!(exports.is_empty(), expected_test, "{cfg}");
    }
}

#[test]
fn cargo_target_kinds_keep_custom_executable_and_test_roots_distinct() {
    let mut f = Fixture::new();
    f.member("sample", "", "");
    f.write("sample/Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'\nbuild='custom/build.rs'\n[[bin]]\nname='tool'\npath='custom/entry.rs'\n[[test]]\nname='fixture'\npath='custom/check.rs'\n");
    f.write("sample/custom/entry.rs", "mod sibling; fn main() {} ");
    f.write("sample/custom/sibling.rs", "pub struct Production;");
    f.write("sample/custom/check.rs", "pub struct TestOnly;");
    f.write("sample/custom/build.rs", "fn main() {}");
    let ws = f.load();
    let member = ws.member("sample").expect("member");
    for kind in ["lib", "bin", "test", "custom-build"] {
        assert!(
            member
                .targets
                .iter()
                .any(|t| t.kinds.iter().any(|k| k == kind)),
            "{kind}"
        );
    }
    let records = &ws.role_index().expect("index").records;
    assert!(
        records
            .iter()
            .any(|r| r.id.ends_with("::Production") && !r.test)
    );
    assert!(
        records
            .iter()
            .any(|r| r.id.ends_with("::TestOnly") && r.test)
    );
}

#[test]
fn shared_integration_helpers_remain_test_only_with_canonical_paths() {
    let mut f = Fixture::new();
    f.member("sample", "", "");
    f.write("sample/Cargo.toml", "[package]\nname='sample'\nversion='0.1.0'\n[[test]]\nname='first'\npath='tests/first/main.rs'\n[[test]]\nname='second'\npath='tests/second/main.rs'\n");
    for target in ["first", "second"] {
        f.write(
            &format!("sample/tests/{target}/main.rs"),
            "#[path=\"../common/mod.rs\"] mod common;",
        );
    }
    f.write(
        "sample/tests/common/mod.rs",
        "mod child; pub fn setup() {} pub struct Fixture;",
    );
    f.write("sample/tests/common/child.rs", "pub fn helper() {}");
    let ws = f.load();
    let index = ws.role_index().expect("index");
    let helpers: Vec<_> = index
        .records
        .iter()
        .filter(|r| r.file.ends_with("tests/common/mod.rs"))
        .collect();
    assert!(!helpers.is_empty());
    assert!(helpers.iter().all(|r| r.test && !r.file.contains("../")));
    assert!(declaration_only_roots::check(&ws).expect("rule").is_empty());
}
