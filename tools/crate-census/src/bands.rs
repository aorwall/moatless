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
    /// A `#[cfg(test)]`-only backend-local module (test scaffolding, e.g.
    /// `test_support`). It ships in no target crate — it stays in the backend for
    /// its own tests and is never carved. Distinct from `Fallthrough` so membership
    /// reports it as test scaffolding, not as `UNASSIGNED` (nobody classified it).
    TestScaffold,
    Fallthrough,
}

impl CrateAssignment {
    /// The crate id used for edge classification: an explicit arm's id, or `server`
    /// for a fallthrough. Identical to an explicit `server` match, so adding or
    /// removing a `server` arm cannot move the illegal count. Test scaffolding
    /// scores as `server` too, but its refs are demoted to test-context before
    /// scoring, so this only ever labels its membership row.
    pub fn edge_crate(&self) -> &'static str {
        match self {
            CrateAssignment::Explicit(id) => id,
            CrateAssignment::TestScaffold | CrateAssignment::Fallthrough => "server",
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
        // -- core is now the `moatless-core` crate (crates/core), no longer in
        // backend/src. Its modules (config, db, error, events, feature_flags,
        // git_provider, github_client, leader, metrics, scope, shared_models, tag,
        // task_listing) are scored via `external_crate("moatless_core")`. The one
        // former-core file that stays here is `test_support` (below); `""` is the
        // backend lib root, which carves last as `server`. --
        "test_support" => return CrateAssignment::TestScaffold,
        // -- identity --
        // `secret` is core-clean storage/policy but its `handlers.rs` takes `AuthUser`,
        // so by the predicate above it lands in identity (its consumers include auth).
        // `setup` (SetupContext, evaluate_setup) is read only by `auth`, and its own
        // outgoing surface is auth/secret/core — nothing above identity — so it sits here.
        "extractors" | "auth" | "team" | "adapter_apps" | "secret" | "setup" => "identity",
        // -- workspaces --
        "workspace" | "repository" | "skill" | "plugin" | "credentials" => "workspaces",
        // -- tasks --
        // `file_storage` and `task_event` are core-clean storage whose `handlers.rs`
        // take `AuthUser` (and `task_event`'s a `TaskDao`); their consumers are all
        // task/message-adjacent, so they land here (Ruling C).
        // `credential_sync` (9c) is the server-of-a-lower-band orchestrator for the
        // codex refresh/broadcast: its deps floor at `TaskDao` (tasks, 5) and its
        // caller ceiling is `task/service` (also tasks, 5), so the intersection is a
        // single band — the tasks crate is the only home that needs no inversion
        // trait. Without this arm it would fall through to `server` (see `edge_crate`)
        // and its sub-server callers (`sandbox/secrets`, `task/service`) would name it
        // upward — the trap this placement avoids.
        "task" | "message" | "task_activities" | "task_turn" | "agent" | "event_ingress"
        | "feedback" | "agent_event" | "file_storage" | "task_event" | "credential_sync" => {
            "tasks"
        }
        // -- sandbox split --
        "sandbox" => match m.get(1).map(String::as_str) {
            Some("k8s") => "sandbox-k8s",
            Some("docker") => "sandbox-docker",
            _ => "sandbox-control",
        },
        // -- loops --
        "loops" | "bindings" => "loops",
        // -- adapters split: providers -> provider crates, routes -> server, rest -> loops --
        "adapters" => match (m.get(1).map(String::as_str), m.get(2).map(String::as_str)) {
            (Some("providers"), Some("github_pr")) => "adapter-github-pr",
            (Some("providers"), Some("linear")) => "adapter-linear",
            (Some("providers"), Some("slack")) => "adapter-slack",
            (Some("providers"), Some("telegram")) => "adapter-telegram",
            (Some("providers"), Some("webhook")) => "adapter-webhook",
            // `routes.rs` is the adapter API router + OpenAPI doc: `adapter_router`
            // takes `AppState` and composes every provider handler into an axum
            // `Router` (`.with_state(state)`), and `AdapterApiDoc` aggregates their
            // OpenAPI paths. Its entire outgoing set — the five provider crates
            // (band 9), `bindings::handlers` (loops, 8), `handlers` (loops, 8),
            // `AppState` (server, 10) — sits at or below server, and its only two
            // production consumers are server-band (`adapter_router` at main.rs:463,
            // `AdapterApiDoc` at startup.rs:742/827; the remaining callers are
            // dev-dependency test edges). Every reference sits at or below its
            // consumer's band, so this is a placement, not a seam (the 8h
            // precedent): its home is server, where each provider reference runs
            // downward-legal. Carves out of `adapters/` into the server crate at
            // carve time; the arm reflects that home ahead of the `git mv`, exactly
            // as the `setup`/`repository_sync`/`credential_sync` arms do.
            (Some("routes"), _) => "server",
            _ => "loops",
        },
        // -- server -- (`""` is the backend lib root, lib.rs)
        // `repository_sync` is the server-band home of the repository config-sync
        // endpoint: it composes `loops` (LoopSyncService) with `workspaces` (git
        // sync) synchronously, and naming `loops` from `repository` (workspaces)
        // would trap that module — so the cross-band orchestration lives here. The
        // reverse of `setup` moving OUT of this arm into identity.
        "main" | "startup" | "state" | "supervisor" | "compose" | "sse" | "t3code"
        | "event_handler" | "provider_settings" | "task_connections" | "repository_sync"
        | "" => "server",
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
/// `core` (band 1), `adapter-model` (band 2), and `identity` (band 3), which are
/// real banded crates in the target graph.
pub fn floor_crate_of_dir(dir_name: &str) -> String {
    match dir_name {
        "adapter-model" => "adapter-model".to_string(),
        // `core` is a real band-1 crate (crates/core), not the floor, so it keeps its
        // band id — like `adapter-model`, and unlike the floor `crates/*`.
        "core" => "core".to_string(),
        // `identity` (10b) is a real band-3 crate (crates/identity), not the floor.
        // Mapping it to `floor:identity` instead does NOT inflate the count: a
        // `floor:` id is excluded from scoring entirely by `classify_edge`'s floor
        // guard (model.rs), which returns before any band comparison — so no
        // `0 <= band` arithmetic is ever reached. The real failure is worse and
        // silent: identity would leave the census graph. Its 68 files would score as
        // floor (excluded), and every backend `moatless_identity::` reference would
        // hit `external_crate`'s `_ => None` (no census edge), so a whole band-3
        // crate becomes invisible to the ratchet — its 0-in/0-out property no longer
        // measured, a future illegal edge into or out of it never caught. Inert until
        // crates/identity exists (the 10b move).
        "identity" => "identity".to_string(),
        // `workspaces` (10c) is a real band-4 crate (crates/workspaces), not the
        // floor. Same reasoning as `identity` above: mapping it to `floor:workspaces`
        // would drop a whole band-4 crate out of the scored graph — its files score as
        // floor (excluded), every backend `moatless_workspaces::` reference misses
        // `external_crate` (no census edge), and its 0-out property stops being
        // measured. Inert until crates/workspaces exists (the 10c move). The
        // carved-but-unbanded guard in main.rs is the backstop for the day this arm is
        // forgotten: it fires the instant a crates/<dir> whose name is a real band
        // still resolves to `floor:<dir>`.
        "workspaces" => "workspaces".to_string(),
        // `tasks` (10d) is a real band-5 crate (crates/tasks), not the floor. Same
        // reasoning as `identity`/`workspaces` above: mapping it to `floor:tasks`
        // drops a whole band-5 crate out of the scored graph — its files score as
        // floor (excluded), every backend `moatless_tasks::` facade re-export misses
        // `external_crate` (no census edge), and — worse — its outgoing edges launder
        // to `floor -> X` (downward-legal), so a future illegal edge out of `tasks`
        // is never caught. 10d carved the crate physically but this arm was skipped;
        // the carved-but-unbanded backstop in main.rs flagged it, and this arm is the
        // fix. A physical carve without its precursor arm silently unbands the crate.
        "tasks" => "tasks".to_string(),
        // `sandbox-control` (10e) is a real band-6 crate (crates/sandbox-control),
        // not the floor. Same reasoning as `tasks` above: mapping it to
        // `floor:sandbox-control` drops a whole band-6 crate out of the scored graph
        // — its files score as floor (excluded), every backend
        // `moatless_sandbox_control::` facade re-export misses `external_crate` (no
        // census edge), and its outgoing edges launder to `floor -> X`
        // (downward-legal), so a future illegal edge out of it is never caught. The
        // arm ships in the same slice as the `git mv`; the carved-but-unbanded
        // backstop in main.rs is the tripwire if it is ever forgotten.
        "sandbox-control" => "sandbox-control".to_string(),
        // `sandbox-k8s` (10f) is a real band-7 crate (crates/sandbox-k8s), not the
        // floor. Same reasoning as `sandbox-control` above: mapping it to
        // `floor:sandbox-k8s` drops a whole band-7 crate out of the scored graph —
        // its files score as floor (excluded) and its outgoing edges launder to
        // `floor -> X` (downward-legal), so a future illegal edge out of it (e.g. an
        // upward reference into `docker` at the same band, or into a server-band
        // module) is never caught. The arm ships in the same slice as the `git mv`;
        // the carved-but-unbanded backstop in main.rs is the tripwire if forgotten.
        "sandbox-k8s" => "sandbox-k8s".to_string(),
        // `sandbox-docker` (10f) is a real band-7 crate (crates/sandbox-docker), the
        // sibling of `sandbox-k8s` at the same band. Same reasoning as above: without
        // the arm its files score as `floor:sandbox-docker` (excluded) and its
        // outgoing edges launder to downward-legal, so a future illegal edge out of
        // it (e.g. an upward reference into `sandbox-k8s` at the same band) is never
        // caught. The arm ships in the same slice as the `git mv`.
        "sandbox-docker" => "sandbox-docker".to_string(),
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
        "moatless_core" => "core",
        "moatless_api_schemas" => "floor:api-schemas",
        "moatless_sandbox_proto" => "floor:sandbox-proto",
        "moatless_cli" => "floor:cli",
        "moatless_api_client" => "floor:api-client",
        "moatless_t3code" => "floor:t3code",
        "claude_agent_sdk" => "floor:claude-agent-sdk",
        "moatless_adapter_model" => "adapter-model",
        // `moatless_identity` (10b) resolves to the band-3 identity crate, so a
        // backend (band 4..10) reference to it scores as a downward-legal edge and
        // identity's own downward deps stay legal. Mirrors `core`/`adapter-model`.
        // Without this arm the head falls to `_ => None` below — "no census edge" —
        // so every backend->identity reference is silently dropped rather than
        // scored, the export half of identity leaving the census graph. Inert until
        // something names `moatless_identity::` (the 10b move).
        "moatless_identity" => "identity",
        // `moatless_workspaces` (10c) resolves to the band-4 workspaces crate, so a
        // backend (band 5..10) reference to it scores as a downward-legal edge and
        // workspaces' own downward deps (identity, core) stay legal. Mirrors
        // `moatless_identity`. Without this arm the head falls to `_ => None` — "no
        // census edge" — and every backend->workspaces reference is silently dropped
        // rather than scored, the export half of workspaces leaving the census graph.
        // Inert until something names `moatless_workspaces::` (the 10c move).
        "moatless_workspaces" => "workspaces",
        // `moatless_tasks` (10d) resolves to the band-5 tasks crate, so a backend
        // (band 6..10) reference to it scores as a downward-legal edge and tasks'
        // own downward deps stay legal. Mirrors `moatless_identity`/`moatless_workspaces`.
        // Without this arm the head falls to `_ => None` — "no census edge" — and
        // every backend `moatless_tasks::` facade re-export (feedback, agent, message,
        // event_ingress, task_event, task_turn, file_storage, credential_sync, …) is
        // silently dropped rather than scored, the export half of tasks leaving the
        // census graph. Skipped at the 10d carve; restored here.
        "moatless_tasks" => "tasks",
        // `moatless_sandbox_control` (10e) resolves to the band-6 sandbox-control
        // crate, so a backend (band 7..10) reference to it — every `crate::sandbox::`
        // access routed through the facade at src/sandbox/mod.rs, including the k8s
        // and docker providers at band 7 — scores as a downward-legal edge, and the
        // crate's own downward deps stay legal. Without this arm the head falls to
        // `_ => None` ("no census edge") and the export half of sandbox-control
        // leaves the census graph. Mirrors `moatless_tasks`.
        "moatless_sandbox_control" => "sandbox-control",
        // `moatless_sandbox_k8s` (10f) resolves to the band-7 sandbox-k8s crate, so a
        // backend (band 8..10) reference to it — every `crate::sandbox::k8s::` access
        // routed through the `pub use moatless_sandbox_k8s as k8s` facade alias —
        // scores as a downward-legal edge, and the crate's own downward deps
        // (sandbox-control, tasks, workspaces, core, …) stay legal. Without this arm
        // the head falls to `_ => None` ("no census edge") and the export half of
        // sandbox-k8s leaves the census graph. Mirrors `moatless_sandbox_control`.
        "moatless_sandbox_k8s" => "sandbox-k8s",
        // `moatless_sandbox_docker` (10f) resolves to the band-7 sandbox-docker
        // crate, so a backend (band 8..10) reference to it — every
        // `crate::sandbox::docker::` access routed through the
        // `pub use moatless_sandbox_docker as docker` facade alias — scores as a
        // downward-legal edge, and the crate's own downward deps stay legal. Without
        // this arm the head falls to `_ => None` ("no census edge") and the export
        // half of sandbox-docker leaves the census graph. Mirrors `moatless_sandbox_k8s`.
        "moatless_sandbox_docker" => "sandbox-docker",
        _ => return None,
    };
    Some(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the BAND, not the arm's existence: `floor_crate_of_dir("identity")`
    /// returning the fallthrough `floor:identity` would *pass* an "is non-empty"
    /// check while dropping identity out of the scored graph entirely (a `floor:`
    /// id is excluded by `classify_edge`'s floor guard, so it is never scored at
    /// all). Assert it lands at band 3 so that silent mismodel trips the test here
    /// rather than making a whole band-3 crate invisible in the 10b move.
    #[test]
    fn identity_dir_resolves_to_the_band_3_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("identity"), "identity");
        assert_eq!(
            band_of(&floor_crate_of_dir("identity")),
            3,
            "crates/identity must be the band-3 identity crate, not floor band 0"
        );
        // Guard the trap explicitly: the fallthrough it must NOT take is band 0.
        assert_eq!(band_of("floor:identity"), 0);
    }

    /// A backend reference to `moatless_identity::` must resolve to the band-3
    /// identity crate (so it scores downward-legal), the same way `moatless_core`
    /// and `moatless_adapter_model` resolve to their bands.
    #[test]
    fn moatless_identity_head_resolves_to_the_band_3_crate() {
        assert_eq!(external_crate("moatless_identity"), Some("identity".to_string()));
        let Some(id) = external_crate("moatless_identity") else {
            panic!("moatless_identity must be a known external head");
        };
        assert_eq!(band_of(&id), 3, "moatless_identity must resolve to band-3 identity");
    }

    /// The 10c analogue of the identity band-pin: `crates/workspaces` must resolve to
    /// the band-4 workspaces crate, not the `floor:workspaces` fallthrough (band 0,
    /// excluded from scoring). Asserting the band, not just non-emptiness, is what
    /// makes the fallthrough a test failure rather than a silent whole-crate drop.
    #[test]
    fn workspaces_dir_resolves_to_the_band_4_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("workspaces"), "workspaces");
        assert_eq!(
            band_of(&floor_crate_of_dir("workspaces")),
            4,
            "crates/workspaces must be the band-4 workspaces crate, not floor band 0"
        );
        // The trap it must NOT take is band 0.
        assert_eq!(band_of("floor:workspaces"), 0);
    }

    /// A backend reference to `moatless_workspaces::` must resolve to the band-4
    /// workspaces crate (so it scores downward-legal), mirroring `moatless_identity`.
    #[test]
    fn moatless_workspaces_head_resolves_to_the_band_4_crate() {
        assert_eq!(external_crate("moatless_workspaces"), Some("workspaces".to_string()));
        let Some(id) = external_crate("moatless_workspaces") else {
            panic!("moatless_workspaces must be a known external head");
        };
        assert_eq!(band_of(&id), 4, "moatless_workspaces must resolve to band-4 workspaces");
    }

    /// The 10d analogue: `crates/tasks` must resolve to the band-5 tasks crate, not
    /// the `floor:tasks` fallthrough (band 0, excluded from scoring). This is the
    /// arm 10d's carve skipped — asserting the band, not just non-emptiness, is what
    /// turns the fallthrough into a test failure rather than a silent whole-crate drop.
    #[test]
    fn tasks_dir_resolves_to_the_band_5_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("tasks"), "tasks");
        assert_eq!(
            band_of(&floor_crate_of_dir("tasks")),
            5,
            "crates/tasks must be the band-5 tasks crate, not floor band 0"
        );
        // The trap it must NOT take is band 0.
        assert_eq!(band_of("floor:tasks"), 0);
    }

    /// A backend reference to `moatless_tasks::` must resolve to the band-5 tasks
    /// crate (so it scores downward-legal), mirroring `moatless_identity`/`moatless_workspaces`.
    #[test]
    fn moatless_tasks_head_resolves_to_the_band_5_crate() {
        assert_eq!(external_crate("moatless_tasks"), Some("tasks".to_string()));
        let Some(id) = external_crate("moatless_tasks") else {
            panic!("moatless_tasks must be a known external head");
        };
        assert_eq!(band_of(&id), 5, "moatless_tasks must resolve to band-5 tasks");
    }

    /// The 10e analogue: `crates/sandbox-control` must resolve to the band-6
    /// sandbox-control crate, not the `floor:sandbox-control` fallthrough (band 0,
    /// excluded from scoring). Asserting the band, not just non-emptiness, turns the
    /// fallthrough into a test failure rather than a silent whole-crate drop.
    #[test]
    fn sandbox_control_dir_resolves_to_the_band_6_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("sandbox-control"), "sandbox-control");
        assert_eq!(
            band_of(&floor_crate_of_dir("sandbox-control")),
            6,
            "crates/sandbox-control must be the band-6 sandbox-control crate, not floor band 0"
        );
        // The trap it must NOT take is band 0.
        assert_eq!(band_of("floor:sandbox-control"), 0);
    }

    /// A backend reference to `moatless_sandbox_control::` must resolve to the band-6
    /// sandbox-control crate (so it scores downward-legal), mirroring `moatless_tasks`.
    #[test]
    fn moatless_sandbox_control_head_resolves_to_the_band_6_crate() {
        assert_eq!(
            external_crate("moatless_sandbox_control"),
            Some("sandbox-control".to_string())
        );
        let Some(id) = external_crate("moatless_sandbox_control") else {
            panic!("moatless_sandbox_control must be a known external head");
        };
        assert_eq!(band_of(&id), 6, "moatless_sandbox_control must resolve to band-6 sandbox-control");
    }

    /// The 10f analogue: `crates/sandbox-k8s` must resolve to the band-7 sandbox-k8s
    /// crate, not the `floor:sandbox-k8s` fallthrough (band 0, excluded from
    /// scoring). Asserting the band, not just non-emptiness, turns the fallthrough
    /// into a test failure rather than a silent whole-crate drop.
    #[test]
    fn sandbox_k8s_dir_resolves_to_the_band_7_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("sandbox-k8s"), "sandbox-k8s");
        assert_eq!(
            band_of(&floor_crate_of_dir("sandbox-k8s")),
            7,
            "crates/sandbox-k8s must be the band-7 sandbox-k8s crate, not floor band 0"
        );
        // The trap it must NOT take is band 0.
        assert_eq!(band_of("floor:sandbox-k8s"), 0);
    }

    /// A backend reference to `moatless_sandbox_k8s::` must resolve to the band-7
    /// sandbox-k8s crate (so it scores downward-legal), mirroring `moatless_sandbox_control`.
    #[test]
    fn moatless_sandbox_k8s_head_resolves_to_the_band_7_crate() {
        assert_eq!(
            external_crate("moatless_sandbox_k8s"),
            Some("sandbox-k8s".to_string())
        );
        let Some(id) = external_crate("moatless_sandbox_k8s") else {
            panic!("moatless_sandbox_k8s must be a known external head");
        };
        assert_eq!(band_of(&id), 7, "moatless_sandbox_k8s must resolve to band-7 sandbox-k8s");
    }

    /// The 10f docker analogue: `crates/sandbox-docker` must resolve to the band-7
    /// sandbox-docker crate, not the `floor:sandbox-docker` fallthrough (band 0,
    /// excluded from scoring).
    #[test]
    fn sandbox_docker_dir_resolves_to_the_band_7_crate_not_the_floor() {
        assert_eq!(floor_crate_of_dir("sandbox-docker"), "sandbox-docker");
        assert_eq!(
            band_of(&floor_crate_of_dir("sandbox-docker")),
            7,
            "crates/sandbox-docker must be the band-7 sandbox-docker crate, not floor band 0"
        );
        // The trap it must NOT take is band 0.
        assert_eq!(band_of("floor:sandbox-docker"), 0);
    }

    /// A backend reference to `moatless_sandbox_docker::` must resolve to the band-7
    /// sandbox-docker crate (so it scores downward-legal), mirroring `moatless_sandbox_k8s`.
    #[test]
    fn moatless_sandbox_docker_head_resolves_to_the_band_7_crate() {
        assert_eq!(
            external_crate("moatless_sandbox_docker"),
            Some("sandbox-docker".to_string())
        );
        let Some(id) = external_crate("moatless_sandbox_docker") else {
            panic!("moatless_sandbox_docker must be a known external head");
        };
        assert_eq!(band_of(&id), 7, "moatless_sandbox_docker must resolve to band-7 sandbox-docker");
    }
}
