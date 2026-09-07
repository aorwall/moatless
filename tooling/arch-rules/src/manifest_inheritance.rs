//! Package identity and lint configuration come from the workspace manifest.

use crate::Violation;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "manifest-inheritance";

fn missing(manifest: &toml::Table) -> Vec<&'static str> {
    ["version", "edition", "authors", "lints"]
        .into_iter()
        .filter(|key| {
            let value = if *key == "lints" {
                manifest.get(*key)
            } else {
                manifest.get("package").and_then(|p| p.get(*key))
            };
            !value
                .and_then(|v| v.get("workspace"))
                .and_then(toml::Value::as_bool)
                .unwrap_or(false)
        })
        .collect()
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    Ok(ws.members()
        .flat_map(|m| {
            missing(&m.manifest).into_iter().map(move |key| {
                Violation::at(
                    RULE,
                    format!("{}: {key}", m.name),
                    format!("{key} must inherit from the workspace"),
                    ws.relative(&m.dir.join("Cargo.toml")),
                )
            })
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "fixtures must parse")]
mod tests {
    use super::*;
    #[test]
    fn resolved_values_do_not_prove_inheritance() {
        let direct = "[package]\nversion = '0.1.0'\nedition = '2021'\nauthors = []\n[lints]\nworkspace = true".parse().unwrap();
        assert_eq!(missing(&direct), vec!["version", "edition", "authors"]);
        let inherited = "[package]\nversion.workspace = true\nedition.workspace = true\nauthors.workspace = true\n[lints]\nworkspace = true".parse().unwrap();
        assert!(missing(&inherited).is_empty());
    }
}
