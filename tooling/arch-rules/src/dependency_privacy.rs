//! A crate may not name a dependency its band forbids.
//!
//! Cargo enforces acyclicity and nothing else, so band order and driver privacy
//! are held here. Each entry states the rule and the document that explains it,
//! because a forbidden edge is only obvious to whoever wrote it down.

use crate::Violation;
use crate::workspace::Workspace;
use anyhow::Result;

pub const RULE: &str = "dependency-privacy";

struct Forbidden {
    /// The crate that must not reach `target`.
    source: &'static str,
    target: &'static str,
    /// Whether an indirect path counts, or only a declared dependency.
    transitive: bool,
    why: &'static str,
}

/// Privacy constraints that dependency band order cannot express.
const FORBIDDEN: &[Forbidden] = &[
    Forbidden {
        source: "moatless-identity",
        target: "moatless-workspace-types",
        transitive: false,
        why: "identity owns credential contracts; Workspace adapts its values to them",
    },
    Forbidden {
        source: "moatless-identity",
        target: "moatless-workspace-ports",
        transitive: false,
        why: "identity owns credential contracts; Workspace implements the override seam",
    },
    Forbidden {
        source: "moatless-workspaces",
        target: "moatless-api-schemas",
        transitive: false,
        why: "Workspace schemas and shared HTTP types have their own owners — .agents/skills/rust-dev-guidelines/references/file-roles.md",
    },
    Forbidden {
        source: "moatless-adapter-slack",
        target: "moatless-loops",
        transitive: true,
        why: "the Adapter kernel freed this crate from the loops band",
    },
    Forbidden {
        source: "moatless-adapter-webhook",
        target: "moatless-loops",
        transitive: true,
        why: "the Adapter kernel freed this crate from the loops band",
    },
    Forbidden {
        source: "moatless-sandbox-docker",
        target: "moatless-api-schemas",
        transitive: false,
        why: "a runtime driver speaks the runtime contract and the domain floor, never the wire contract",
    },
    Forbidden {
        source: "moatless-sandbox-k8s",
        target: "moatless-api-schemas",
        transitive: false,
        why: "a runtime driver speaks the runtime contract and the domain floor, never the wire contract",
    },
    Forbidden {
        source: "moatless-types",
        target: "sqlx",
        transitive: false,
        why: "the floor stays driver-free",
    },
    Forbidden {
        source: "moatless-api-schemas",
        target: "sqlx",
        transitive: false,
        why: "a wire schema owns no table",
    },
    Forbidden {
        source: "moatless-sandbox-runtime",
        target: "sqlx",
        transitive: false,
        why: "a runtime contract owns no table",
    },
    Forbidden {
        source: "moatless-adapter-slack",
        target: "sqlx",
        transitive: false,
        why: "a provider owns no table",
    },
    Forbidden {
        source: "moatless-adapter-webhook",
        target: "sqlx",
        transitive: false,
        why: "a provider owns no table",
    },
    Forbidden {
        source: "moatless-adapter-linear",
        target: "sqlx",
        transitive: false,
        why: "a provider owns no table",
    },
    Forbidden {
        source: "moatless-adapter-github-pr",
        target: "sqlx",
        transitive: false,
        why: "a provider owns no table",
    },
    Forbidden {
        source: "moatless-decisions",
        target: "sqlx",
        transitive: false,
        why: "a provider owns no table",
    },
    Forbidden {
        source: "moatless-ports",
        target: "sandbox-proto",
        transitive: false,
        why: "a shared port vocabulary names no transport",
    },
    Forbidden {
        source: "moatless-core",
        target: "moatless-api-schemas",
        transitive: false,
        why: "the lowest band names no wire schema",
    },
];

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut found: Vec<Violation> = FORBIDDEN
        .iter()
        .filter(|rule| {
            ws.member(rule.source)
                .is_some_and(|m| !crate::annotations::permits_dependency(m, RULE, rule.target))
        })
        .filter(|rule| {
            if rule.transitive {
                ws.reaches(rule.source, rule.target)
            } else {
                ws.member(rule.source)
                    .is_some_and(|m| m.normal_deps.contains(rule.target))
            }
        })
        .map(|rule| Violation {
            rule: RULE,
            key: format!("{} -> {}", rule.source, rule.target),
            message: format!(
                "{} depends on {}{}: {}",
                rule.source,
                rule.target,
                if rule.transitive {
                    " (transitively)"
                } else {
                    ""
                },
                rule.why
            ),
            locations: vec![format!(
                "crates/{}/Cargo.toml",
                rule.source.trim_start_matches("moatless-")
            )],
        })
        .collect();
    let index = ws.index()?;
    for item in &index.records {
        let restricted = matches!(
            item.package.as_str(),
            "moatless-workspace-ports" | "moatless-sandbox-docker" | "moatless-sandbox-k8s"
        ) || (item.package == "moatless-workspace-types"
            && !(item.module == "moatless_workspace_types::schemas"
                || item
                    .module
                    .starts_with("moatless_workspace_types::schemas::")));
        if restricted && !item.test {
            for path in index
                .paths(item)
                .filter(|p| p.starts_with("moatless_workspace_types::schemas::"))
            {
                // A root may declare the optional namespace, but cannot forward its contents.
                if matches!(item.item, syn::Item::Mod(_)) {
                    continue;
                }
                found.push(Violation::at(
                    RULE,
                    format!("{} names {path}", item.id),
                    "domain values, ports, and runtime drivers cannot name wire schemas",
                    &item.file,
                ));
            }
        }
    }
    Ok(found)
}
