//! The target-crate partition and band order.
//!
//! Authoritative source: `.plans/backend-ports-and-crates.md` — the 15-crate
//! table (:526-542) and the band order (:573-583). One deliberate divergence,
//! recorded here because the plan flags it as unconfirmed (:585): `adapter-model`
//! is placed strictly *below* `identity`, not beside it as a sibling, because
//! the Step 9f ruling is that `identity -> adapter-model` is downward-legal.
//! The census confirms or refutes that by measuring `adapter-model -> identity`
//! (expected zero).

use crate::model::ModPath;

/// Census roots and the scope each carries.
pub const CENSUS_ROOTS: &[(&str, ScopeKind)] = &[
    ("backend/src", ScopeKind::Backend),
    ("backend/tests", ScopeKind::BackendTest),
    ("crates", ScopeKind::Floor), // src and tests distinguished per-file
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKind {
    Backend,
    BackendTest,
    Floor,
    FloorTest,
}

/// Strict band index (higher depends on lower; a legal edge runs high -> low).
/// `adapter-model` sits between `core` and `identity` (see module doc).
pub fn band_of(crate_id: &str) -> i32 {
    if crate_id.starts_with("floor:") {
        return 0;
    }
    match crate_id {
        "core" => 1,
        "adapter-model" => 2,
        "identity" => 3,
        "workspaces" => 4,
        "tasks" => 5,
        "sandbox-control" => 6,
        "sandbox-k8s" | "sandbox-docker" => 7,
        "loops" => 8,
        "adapter-github-pr" | "adapter-linear" | "adapter-slack" | "adapter-telegram"
        | "adapter-webhook" => 9,
        "server" => 10,
        // test source crates sit at the top: their refs are dev-dependency edges.
        "test:backend" => 100,
        _ => 50, // unknown — should not happen; keeps it out of illegal by being high
    }
}

/// The outcome of mapping a backend module to a target crate. `Explicit` is a
/// module an arm of `crate_of_backend_mod` names; `Fallthrough` is the catch-all —
/// a top-level module no arm names. Edge resolution treats the two identically (a
/// fallthrough resolves to `server` via `edge_crate`, byte-for-byte like an explicit
/// `server` arm), so the illegal-edge count never depends on the distinction.
/// Membership aggregation keeps them apart: a fallthrough module lands in the
/// report's UNASSIGNED section rather than folded silently into `server`, so the
/// generated output can say "I do not know where this belongs" instead of guessing.
pub enum CrateAssignment {
    Explicit(&'static str),
    Fallthrough,
}

impl CrateAssignment {
    /// The crate id used for edge classification: an explicit arm's id, or `server`
    /// for a fallthrough. Identical to an explicit `server` match, so adding or
    /// removing a `server` arm cannot move the illegal count.
    pub fn edge_crate(&self) -> &'static str {
        match self {
            CrateAssignment::Explicit(id) => id,
            CrateAssignment::Fallthrough => "server",
        }
    }
}

/// The 15-crate partition for a backend module path (rooted at `lib.rs`). A
/// top-level module no arm names returns `Fallthrough` (see `CrateAssignment`).
pub fn crate_of_backend_mod(m: &ModPath) -> CrateAssignment {
    let top = m.first().map_or("", String::as_str);
    // Root files (len 1) and directory modules both key on the first segment,
    // except `adapters` and `sandbox`, which split on the second.
    let id = match top {
        // -- core --
        "config" | "db" | "error" | "events" | "feature_flags" | "metrics" | "scope"
        | "shared_models" | "test_support" | "git_provider" | "github_client" | "" => "core",
        // A module with an authenticated HTTP handler cannot be core: `AuthUser` is
        // band-identity by construction — a `FromRequestParts` extractor received per
        // request, which cannot be inverted behind a port without the handler leaving
        // the HTTP layer. So core may hold a module with no handler (`task_listing`) or
        // an unauthenticated one (`tag`, which lists a global vocabulary), and nothing
        // else. Ruling C moved `file_storage` and `task_event` to tasks and `secret` to
        // identity for exactly this reason; check the predicate in one grep for
        // `AuthUser` before adding a module here.
        "tag" | "task_listing" => "core",
        // `leader` is the dependency-free LeaderGuard trait (K8sLeaderGuard lives
        // under sandbox/k8s); Ruling A moved it down so its consumers depend on core.
        "leader" => "core",
        // -- identity --
        // `secret` is core-clean storage/policy but its `handlers.rs` takes `AuthUser`,
        // so by the predicate above it lands in identity (its consumers include auth).
        "extractors" | "auth" | "team" | "adapter_apps" | "secret" => "identity",
        // -- workspaces --
        "workspace" | "repository" | "skill" | "plugin" | "credentials" => "workspaces",
        // -- tasks --
        // `file_storage` and `task_event` are core-clean storage whose `handlers.rs`
        // take `AuthUser` (and `task_event`'s a `TaskDao`); their consumers are all
        // task/message-adjacent, so they land here (Ruling C).
        "task" | "message" | "task_activities" | "task_turn" | "agent" | "event_ingress"
        | "feedback" | "agent_event" | "file_storage" | "task_event" => "tasks",
        // -- sandbox split --
        "sandbox" => match m.get(1).map(String::as_str) {
            Some("k8s") => "sandbox-k8s",
            Some("docker") => "sandbox-docker",
            _ => "sandbox-control",
        },
        // -- loops --
        "loops" | "bindings" => "loops",
        // -- adapters split: providers -> provider crates, rest -> loops --
        "adapters" => match (m.get(1).map(String::as_str), m.get(2).map(String::as_str)) {
            (Some("providers"), Some("github_pr")) => "adapter-github-pr",
            (Some("providers"), Some("linear")) => "adapter-linear",
            (Some("providers"), Some("slack")) => "adapter-slack",
            (Some("providers"), Some("telegram")) => "adapter-telegram",
            (Some("providers"), Some("webhook")) => "adapter-webhook",
            _ => "loops",
        },
        // -- server --
        "main" | "startup" | "state" | "supervisor" | "compose" | "sse" | "t3code"
        | "event_handler" | "setup" | "provider_settings" | "task_connections" => "server",
        // the synthetic bin root
        "\u{0}bin" => "server",
        // Unknown top-level module: no arm claims it. Edge resolution treats it as
        // `server` (see `edge_crate`); membership surfaces it in the report's
        // UNASSIGNED section instead of folding it into `server` unseen.
        _ => return CrateAssignment::Fallthrough,
    };
    CrateAssignment::Explicit(id)
}

/// Existing workspace crates under `crates/`. All are the floor (band 0) except
/// `adapter-model`, which is a real band-2 crate in the target graph.
pub fn floor_crate_of_dir(dir_name: &str) -> String {
    match dir_name {
        "adapter-model" => "adapter-model".to_string(),
        other => format!("floor:{other}"),
    }
}

/// Maps an external crate-name path head to a crate id, or `None` if it is std /
/// a third-party crate (no census edge). `moatless_vibe_backend` is handled by
/// the caller (it resolves back into the backend module tree).
pub fn external_crate(head: &str) -> Option<String> {
    let id = match head {
        "moatless_events" => "floor:events",
        "moatless_backend_core" => "floor:backend-core",
        "moatless_api_schemas" => "floor:api-schemas",
        "moatless_sandbox_proto" => "floor:sandbox-proto",
        "moatless_cli" => "floor:cli",
        "moatless_api_client" => "floor:api-client",
        "moatless_t3code" => "floor:t3code",
        "claude_agent_sdk" => "floor:claude-agent-sdk",
        "moatless_adapter_model" => "adapter-model",
        _ => return None,
    };
    Some(id.to_string())
}
