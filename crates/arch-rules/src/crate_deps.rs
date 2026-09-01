//! A crate may not name a dependency its band forbids.
//!
//! Cargo enforces acyclicity and nothing else, so band order and driver
//! privacy are held here. Each entry states the rule and the document that
//! explains it, because a forbidden edge is only obvious to whoever wrote it
//! down.

use crate::workspace::Workspace;
use crate::Violation;

pub const RULE: &str = "forbidden-crate-dep";

struct Forbidden {
    /// The crate that must not reach `target`.
    source: &'static str,
    target: &'static str,
    /// Whether an indirect path counts, or only a declared dependency.
    transitive: bool,
    why: &'static str,
}

/// The two provider crates the Adapter kernel freed, and the driver rule.
///
/// The sqlx entries are the inverse of `TABLE_OWNERS` in
/// `scripts/check-crate-seams.sh`: a crate that owns no table has no reason to
/// compile a Postgres driver. Only direct declaration counts, because a crate
/// that reaches sqlx through a table owner is not the one compiling it in.
const FORBIDDEN: &[Forbidden] = &[
    Forbidden {
        source: "moatless-adapter-slack",
        target: "moatless-loops",
        transitive: true,
        why: "the Adapter kernel freed this crate from the loops band — crates/adapter-model/AGENTS.md",
    },
    Forbidden {
        source: "moatless-adapter-webhook",
        target: "moatless-loops",
        transitive: true,
        why: "the Adapter kernel freed this crate from the loops band — crates/adapter-model/AGENTS.md",
    },
    Forbidden {
        source: "moatless-types",
        target: "sqlx",
        transitive: false,
        why: "the floor stays driver-free — crates/types/AGENTS.md",
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
];

pub fn check(ws: &Workspace) -> Vec<Violation> {
    FORBIDDEN
        .iter()
        .filter(|rule| ws.member(rule.source).is_some())
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
                if rule.transitive { " (transitively)" } else { "" },
                rule.why
            ),
            locations: vec![format!(
                "crates/{}/Cargo.toml",
                rule.source.trim_start_matches("moatless-")
            )],
        })
        .collect()
}
