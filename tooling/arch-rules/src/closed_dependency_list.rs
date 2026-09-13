//! The shared values crate has a closed set of value-level dependencies.

use crate::Violation;
use crate::annotations::permits_dependency;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "closed-dependency-list";
pub const ALLOWED: &[&str] = &[
    "serde",
    "serde_json",
    "chrono",
    "thiserror",
    "tracing",
    "url",
    "uuid",
    "utoipa",
];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    Ok(ws
        .members()
        .filter(|m| {
            matches!(
                m.name.as_str(),
                "moatless-types" | "moatless-workspace-types" | "moatless-workspace-ports"
            )
        })
        .flat_map(|m| {
            m.normal_deps
                .iter()
                .filter(|d| !allowed(&m.name, d) && !permits_dependency(m, RULE, d))
                .map(|d| {
                    Violation::at(
                        RULE,
                        format!("{} -> {d}", m.name),
                        "dependency is outside the shared-values allowlist",
                        ws.relative(&m.dir.join("Cargo.toml")),
                    )
                })
        })
        .collect())
}

fn allowed(package: &str, dependency: &str) -> bool {
    ALLOWED.contains(&dependency)
        || (package == "moatless-workspace-types" && dependency == "moatless-types")
        || (package == "moatless-workspace-ports"
            && matches!(
                dependency,
                "moatless-types" | "moatless-workspace-types" | "async-trait" | "mockall"
            ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn value_dependencies_exclude_transports() {
        assert!(ALLOWED.contains(&"serde"));
        for dependency in ["sqlx", "reqwest", "axum", "redis", "moatless-core"] {
            assert!(!ALLOWED.contains(&dependency));
        }
    }
}
