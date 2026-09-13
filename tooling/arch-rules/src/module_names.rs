//! A module is named for its domain, not for the kind of thing it holds.
//!
//! Two shapes break it — a kind name at a crate root, with no domain above it
//! to qualify it, and a junk drawer at any depth. `config`, `error`, `kind` and
//! `delivery` are none of them a domain either, though this check does not
//! catch them: the glossary's `_Avoid_` list carries bare "delivery", because
//! the term is Adapter delivery.

use std::path::Path;

use anyhow::Result;

use crate::Violation;
use crate::workspace::Workspace;

pub const RULE: &str = "generic-module-name";

/// A kind rather than a domain. Legitimate under a domain-named directory,
/// where the directory supplies the domain and the leaf splits the kind.
const KIND: &[&str] = &["models", "schemas", "types", "data", "dto", "entities"];

/// A junk drawer at any depth. Nothing names its domain, so nothing predicts
/// its contents.
const DRAWER: &[&str] = &[
    "utils", "util", "helpers", "helper", "common", "misc", "shared",
];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut paths = Vec::new();
    for member in ws.members() {
        let root = ws.relative(&member.dir);
        paths.extend(
            ws.sources(member)?
                .into_iter()
                .map(|(path, _)| (root.clone(), path)),
        );
    }
    Ok(check_paths(&paths))
}

/// Directories whose layout cargo owns, not the crate's own vocabulary.
///
/// `tests/common` is cargo's place for a shared test helper; renaming it to a
/// domain hides it from every test that needs it.
const FOREIGN_LAYOUT: &[&str] = &["tests", "benches", "examples"];

/// Whether cargo owns this path's layout, which it does only for a directory
/// sitting directly under the member root.
///
/// Matching the name at any depth would exempt a hand-written
/// `src/<domain>/tests/` and the `common` drawer beneath it, which is what
/// [`DRAWER`] exists to catch.
fn cargo_owns_layout(member_root: &str, path: &str) -> bool {
    Path::new(path)
        .strip_prefix(member_root)
        .ok()
        .and_then(|rest| rest.components().next())
        .is_some_and(|c| FOREIGN_LAYOUT.contains(&c.as_os_str().to_string_lossy().as_ref()))
}

fn check_paths(paths: &[(String, String)]) -> Vec<Violation> {
    let mut found = Vec::new();
    for (member_root, path) in paths {
        let file = Path::new(path);
        if cargo_owns_layout(member_root, path) {
            continue;
        }
        let Some((module, parent)) = declaration(file) else {
            continue;
        };

        let reason = if DRAWER.contains(&module.as_str()) {
            Some(format!("`{module}` is a junk drawer, not a domain"))
        } else if KIND.contains(&module.as_str())
            && parent.file_name().is_some_and(|dir| dir == "src")
            && !(module == "schemas"
                && parent
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| {
                        name == "workspace-types" || name == "moatless-workspace-types"
                    }))
        {
            Some(format!(
                "`{module}` sits at the crate root with no domain to qualify it"
            ))
        } else {
            None
        };

        if let Some(message) = reason {
            found.push(Violation {
                rule: RULE,
                // The module, not the file. `types.rs` and `types/mod.rs` are
                // one module, so they must not be two baseline slots that a
                // layout change moves between.
                key: format!("{}/{module}", parent.display()),
                message,
                locations: vec![path.clone()],
            });
        }
    }
    found
}

/// The module a source file declares, and the directory that declares it.
///
/// `src/types.rs` and `src/types/mod.rs` both declare `types` in `src` — a
/// directory module takes its name from the directory, not from `mod`, and a
/// parent of `src` means nothing above supplies a domain.
fn declaration(file: &Path) -> Option<(String, &Path)> {
    let parent = file.parent()?;
    if file.file_name().is_some_and(|name| name == "mod.rs") {
        let name = parent.file_name()?.to_string_lossy().into_owned();
        Some((name, parent.parent()?))
    } else {
        let name = file.file_stem()?.to_string_lossy().into_owned();
        Some((name, parent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Member root is the first two components, matching `crates/<name>`.
    fn keys(paths: &[&str]) -> Vec<String> {
        let pairs: Vec<(String, String)> = paths
            .iter()
            .map(|p| {
                let root = p.split('/').take(2).collect::<Vec<_>>().join("/");
                (root, (*p).to_string())
            })
            .collect();
        check_paths(&pairs).into_iter().map(|v| v.key).collect()
    }

    #[test]
    fn a_hand_written_tests_module_inside_src_is_still_checked() {
        assert_eq!(
            keys(&["crates/x/src/agent/harness/tests/common.rs"]),
            vec!["crates/x/src/agent/harness/tests/common"]
        );
    }

    #[test]
    fn a_kind_name_at_a_crate_root_is_flagged() {
        assert_eq!(keys(&["crates/x/src/types.rs"]), vec!["crates/x/src/types"]);
    }

    /// The finding: the same module written as a directory must not evade the
    /// rule, and must land on the same baseline key.
    #[test]
    fn the_directory_form_of_that_module_is_the_same_violation() {
        assert_eq!(
            keys(&["crates/x/src/types/mod.rs"]),
            vec!["crates/x/src/types"]
        );
    }

    #[test]
    fn a_kind_name_under_a_domain_directory_is_not() {
        assert!(keys(&["crates/x/src/task/models.rs"]).is_empty());
        assert!(keys(&["crates/x/src/task/models/mod.rs"]).is_empty());
    }

    #[test]
    fn a_junk_drawer_is_flagged_at_any_depth() {
        assert_eq!(
            keys(&[
                "crates/x/src/a/b/common.rs",
                "crates/x/src/a/b/common/mod.rs"
            ]),
            vec!["crates/x/src/a/b/common", "crates/x/src/a/b/common"]
        );
    }

    #[test]
    fn a_shared_integration_test_helper_is_not_flagged() {
        assert!(keys(&[
            "crates/x/tests/common/mod.rs",
            "crates/x/benches/shared/mod.rs",
        ])
        .is_empty());
    }

    #[test]
    fn a_domain_name_is_never_flagged() {
        assert!(
            keys(&[
                "crates/x/src/lib.rs",
                "crates/x/src/task.rs",
                "crates/x/src/task/mod.rs",
                "crates/x/src/delivery/admission.rs",
            ])
            .is_empty()
        );
    }
}
