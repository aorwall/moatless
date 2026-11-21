//! The total assignment of workspace members to dependency bands.

use crate::Violation;
use crate::annotations::permits_dependency;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "band-order";

/// Members in one band may depend only on members in lower bands.
pub const BANDS: &[(&[&str], &str)] = &[
    (
        &["moatless-types", "moatless-arch-rules"],
        "shared values and independent tooling",
    ),
    (
        &["moatless-workspace-types"],
        "Workspace values and optional schemas",
    ),
    (&["moatless-workspace-ports"], "Workspace contracts"),
    (
        &[
            "claude-agent-sdk",
            "sandbox-proto",
            "moatless-s3",
            "moatless-supervisor",
        ],
        "leaf protocols and infrastructure",
    ),
    (
        &["moatless-api-schemas", "moatless-sandbox"],
        "wire contracts and the sandbox process",
    ),
    (
        &["moatless-ports", "moatless-core", "moatless-api-client"],
        "ports and process infrastructure",
    ),
    (
        &[
            "moatless-adapter-model",
            "moatless-sandbox-runtime",
            "moatless-cli",
        ],
        "runtime contracts and API consumers",
    ),
    (&["moatless-identity"], "identity services"),
    (&["moatless-workspaces"], "workspace services"),
    (
        &[
            "moatless-tasks",
            "moatless-sandbox-k8s",
            "moatless-sandbox-docker",
            "moatless-sandbox-local",
        ],
        "task services and runtime drivers",
    ),
    (
        &[
            "moatless-loops",
            "moatless-sandbox-control",
            "moatless-t3code",
        ],
        "orchestration and projections",
    ),
    (
        &[
            "moatless-adapter-slack",
            "moatless-adapter-linear",
            "moatless-adapter-github-pr",
            "moatless-decisions",
        ],
        "provider integration",
    ),
    (&["moatless-backend"], "the server composition root"),
];

pub fn band(name: &str) -> Option<usize> {
    BANDS.iter().position(|(names, _)| names.contains(&name))
}

fn forbidden(source: Option<usize>, target: Option<usize>) -> bool {
    matches!((source, target), (Some(s), Some(t)) if s <= t)
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for member in ws.members() {
        let source = band(&member.name);
        let file = ws.relative(&member.dir.join("Cargo.toml"));
        if source.is_none() {
            out.push(Violation::at(
                RULE,
                &member.name,
                "workspace member has no band assignment",
                &file,
            ));
        }
        for dep in &member.normal_deps {
            if ws.member(dep).is_some()
                && forbidden(source, band(dep))
                && !permits_dependency(member, RULE, dep)
            {
                let target = band(dep).unwrap_or_default();
                out.push(Violation::at(
                    RULE,
                    format!("{} -> {dep}", member.name),
                    format!(
                        "{} (band {}) depends on {dep} (band {target}): {}",
                        member.name,
                        source.unwrap_or_default(),
                        BANDS[target].1
                    ),
                    &file,
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_strictly_downward_edges_pass() {
        assert!(!forbidden(Some(8), Some(7)));
        assert!(forbidden(Some(7), Some(7)));
        assert!(forbidden(Some(7), Some(8)));
        assert!(!forbidden(Some(7), None));
    }
    #[test]
    fn driver_to_loops_is_forbidden() {
        assert!(forbidden(
            band("moatless-sandbox-k8s"),
            band("moatless-loops")
        ));
        assert!(band("unassigned").is_none());
    }
}
