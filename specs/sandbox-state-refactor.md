# Collapse the Sandbox State Enum Chain

> **Status:** approved
> **Updated:** 2026-07-28
> **Vocabulary:** CONTEXT.md

## Current solution

The Sandbox display state existed as two maintained enum definitions with the same variants:

| Type | Location | Role before this change |
|---|---|---|
| `SandboxStatus` | `crates/events/src/lib.rs` | Event subtype and shared display-state value |
| `SandboxStatus` | `crates/api-schemas/src/sandbox.rs` | API schema copy of the same display-state value |

Runtime and lifecycle code reached the display state through several independent mappings:

| Source vocabulary | Role | Previous projection |
|---|---|---|
| `PodPhase` | Kubernetes runtime adapter input | Method on `PodPhase` |
| Docker container state string | Docker runtime adapter input | Inline string match in the status resolver |
| `ObservedStatus` | Immediate lifecycle operation outcome | Free helper in sandbox handlers |
| `ObservedStatus` | Immediate lifecycle operation outcome | Free helper in lifecycle event sink |
| Watcher lifecycle subtype string | Watcher publication and duplicate suppression input | Free helper in the Kubernetes watcher |
| `SandboxStatus` | Display state | Defensive narrowing into reconciler observation |

The lifecycle model already distinguished operation outcome from reconciler observation.
That distinction is intentional and remains part of the design.

## Planned solution

The canonical public Sandbox display state is `moatless_events::SandboxStatus`.

`moatless_api_schemas::sandbox::SandboxStatus` is a re-export of that canonical type.
The API schema crate depends on the events crate and forwards its OpenAPI feature to the canonical enum.

Runtime and lifecycle vocabularies remain distinct.
Each vocabulary narrows to the public display state through a method on the source type.

| Source vocabulary | Planned projection |
|---|---|
| `PodPhase` | `PodPhase::sandbox_status()` |
| Docker container state | Typed Docker state parsing, then `DockerContainerState::sandbox_status()` |
| `ObservedStatus` | `ObservedStatus::sandbox_status()` |
| Watcher lifecycle subtype | Typed subtype parsing, then `LifecycleSandboxSubType::sandbox_status()` |
| Reconciler observation | Existing defensive narrowing from display state remains distinct |

### Problem

Maintaining two copies of the same public Sandbox state allows drift.

Mapping Docker and watcher states from raw strings at call sites hides the status policy inside local matches.
That makes new runtime states easy to handle inconsistently.

### Behaviour

| Actor | Behaviour |
|---|---|
| Backend caller | Sandbox status responses keep the same public values as before |
| Event consumer | Sandbox lifecycle event subtype values keep the same public values as before |
| Lifecycle reconciler | Operation outcome, reconciler observation, and watcher transition remain separate vocabularies |
| Maintainer | Adding a state variant requires updating a focused projection match or table test |

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-canonical-display-state-owner) | Canonical display state owner | `moatless_events::SandboxStatus` | A new crate adds dependency churn without solving a current dependency cycle |
| [2](#2-scope-of-one-canonical-state) | Scope of one canonical state | One canonical display/API state | One lifecycle mega-enum would erase useful distinctions |
| [3](#3-projection-location) | Projection location | Methods on source vocabularies | Free helpers near call sites repeat mappings and drift silently |
| [4](#4-validation-seam) | Validation seam | Pure table tests | Runtime or cluster tests add little for deterministic state projections |

### 1. Canonical display state owner

`moatless_events::SandboxStatus` already carried serialization, parsing, display formatting, legacy `fatal` parsing, and OpenAPI support behind a feature.

Re-exporting it from API schemas deletes the duplicate enum while preserving the existing import path for API consumers.

The cost is that `moatless-api-schemas` now depends on `moatless-events`.
That dependency is acceptable because both crates are shared wire-contract crates and no cycle is introduced.

### 2. Scope of one canonical state

The canonical type is the public display/API state only.

`ObservedStatus`, `SandboxObservedStatus`, and `ObservedTransition` remain distinct because each answers a different question:

| Vocabulary | Question it answers |
|---|---|
| `ObservedStatus` | What did this lifecycle operation immediately report? |
| `SandboxObservedStatus` | What observed state should the reconciler join against desired state? |
| `ObservedTransition` | What raw watcher transition should be published or suppressed? |

Flattening those into the display state would reduce locality in lifecycle logic.

### 3. Projection location

Each narrowing lives on the source vocabulary.

That keeps the interface small for callers:
callers ask the source value for its display projection instead of importing a helper and repeating status policy.

The implementation cost is adding small private adapter enums where the source was previously an untyped string.

### 4. Validation seam

The test seam is the projection method.

Every runtime or lifecycle narrowing has a table test that states the complete mapping.
The tests are pure and require no database, Docker daemon, or cluster.

## Seams and testing

| Module | Interface | Depth | Seam | Test leverage |
|---|---|---|---|---|
| Events crate | `SandboxStatus` | Public state owns serialization and parsing | Shared wire-contract type | Existing serialization and legacy parsing tests |
| API schemas crate | Re-exported `SandboxStatus` | Keeps API import path while deleting enum body | Crate dependency seam | API schema test suite |
| Lifecycle types | `ObservedStatus::sandbox_status()` | Operation outcome owns its display projection | Lifecycle outcome seam | Event sink projection test and backend suite |
| Kubernetes models | `PodPhase::sandbox_status()` | Runtime phase owns display projection | Runtime adapter seam | Exhaustive phase table |
| Docker status resolver | `DockerContainerState::try_from` and `sandbox_status()` | Raw Docker state parsing is isolated | Runtime adapter seam | Known-state and unknown-state table tests |
| Kubernetes watcher | `LifecycleSandboxSubType::try_from` and `sandbox_status()` | Raw subtype parsing is isolated | Watcher publication seam | Known-subtype and unknown-subtype table tests |

Validation commands:

| Command | Purpose |
|---|---|
| `cargo clippy -p moatless-api-schemas -- -D warnings` | Shared schema crate lint |
| `cargo test -p moatless-api-schemas` | Shared schema crate tests |
| `cargo clippy -p moatless-vibe-backend -- -D warnings` | Backend lint |
| `cargo test -p moatless-vibe-backend` | Backend unit and integration test suite |

## Out of scope

No public status value is renamed.

No lifecycle behaviour is changed.

No frontend status model is changed beyond whatever the shared generated schema already observes.

No Docker or Kubernetes runtime verification is required for this refactor.

## Open questions

None.

## Ticket breakdown

None.
