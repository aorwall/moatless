//! A module is named for its domain, not for the kind of thing it holds.
//!
//! The rule is stated in `crates/types/AGENTS.md`. Two shapes break it — a
//! kind name at a crate root, with no domain above it to qualify it, and a
//! junk drawer at any depth.

use std::path::Path;

use anyhow::Result;

use crate::workspace::Workspace;
use crate::Violation;

pub const RULE: &str = "generic-module-name";

/// A kind rather than a domain. Legitimate under a domain-named directory,
/// where the directory supplies the domain and the leaf splits the kind.
const KIND: &[&str] = &["models", "schemas", "types", "data", "dto", "entities"];

/// A junk drawer at any depth. Nothing names its domain, so nothing predicts
/// its contents.
const DRAWER: &[&str] = &["utils", "util", "helpers", "helper", "common", "misc", "shared"];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut paths = Vec::new();
    for member in ws.members() {
        paths.extend(ws.sources(member)?.into_iter().map(|(path, _)| path));
    }
    Ok(check_paths(&paths))
}

fn check_paths(paths: &[String]) -> Vec<Violation> {
    let mut found = Vec::new();
    for path in paths {
        let file = Path::new(path);
        let Some((module, parent)) = declaration(file) else {
            continue;
        };

        let reason = if DRAWER.contains(&module.as_str()) {
            Some(format!("`{module}` is a junk drawer, not a domain"))
        } else if KIND.contains(&module.as_str())
            && parent.file_name().is_some_and(|dir| dir == "src")
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
/// `src/types.rs` and `src/types/mod.rs` both declare `types` in `src`, so the
/// rule must see them alike — the name is the module's, and a directory module
/// takes its name from the directory rather than from `mod`. A parent of `src`
/// means nothing above the module supplies a domain.
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

    fn keys(paths: &[&str]) -> Vec<String> {
        check_paths(&paths.iter().map(|p| (*p).to_string()).collect::<Vec<_>>())
            .into_iter()
            .map(|v| v.key)
            .collect()
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
            keys(&["crates/x/src/a/b/common.rs", "crates/x/src/a/b/common/mod.rs"]),
            vec!["crates/x/src/a/b/common", "crates/x/src/a/b/common"]
        );
    }

    #[test]
    fn a_domain_name_is_never_flagged() {
        assert!(keys(&[
            "crates/x/src/lib.rs",
            "crates/x/src/task.rs",
            "crates/x/src/task/mod.rs",
            "crates/x/src/delivery/admission.rs",
        ])
        .is_empty());
    }
}
