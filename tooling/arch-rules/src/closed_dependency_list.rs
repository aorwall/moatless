//! The shared values crate has a closed set of value-level dependencies.

use crate::Violation;
use crate::annotations::permits_dependency;
use crate::workspace::Workspace;

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

pub fn check(ws: &Workspace) -> Vec<Violation> {
    ws.member("moatless-types")
        .into_iter()
        .flat_map(|m| {
            m.normal_deps
                .iter()
                .filter(|d| !ALLOWED.contains(&d.as_str()) && !permits_dependency(m, RULE, d))
                .map(|d| {
                    Violation::at(
                        RULE,
                        format!("{} -> {d}", m.name),
                        "dependency is outside the shared-values allowlist",
                        ws.relative(&m.dir.join("Cargo.toml")),
                    )
                })
        })
        .collect()
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
