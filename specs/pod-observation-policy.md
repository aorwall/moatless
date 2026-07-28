# Pod observation policy

> **Status:** approved
> **Updated:** 2026-07-28
> **Vocabulary:** CONTEXT.md

## Current solution

The Sandbox runtime watcher currently owns both observation mechanics and most observed-state policy.
It receives runtime observations, keeps a per-pod memory of prior state, classifies transitions, publishes Sandbox and Preview server status changes, fetches detail for error messages, and starts remediation.

The existing lifecycle architecture already sets one important constraint:
the watcher keeps crash detection and the disabled-server decision, while lifecycle owns intent and runtime operations.
This spec does not reopen that split.

| Module | Current role |
|---|---|
| `backend/src/sandbox/k8s/pod_watcher.rs::spawn_pod_lifecycle_watcher` | Starts the leader-elected watcher and builds its dependency bundle. |
| `backend/src/sandbox/k8s/pod_watcher.rs::run_pod_watch` | Consumes runtime observations and runs periodic sweeps for slow startup, cleanup, and pending auto-stop retry. |
| `backend/src/sandbox/k8s/pod_watcher.rs::process_watcher_event` | Extracts observed state, detects transitions, publishes events, fetches details, and triggers remediation. |
| `backend/src/sandbox/k8s/pod_watcher.rs::extract_pod_observed_state` | Converts a raw runtime observation into `PodObservedState`. |
| `backend/src/sandbox/k8s/pod_watcher.rs::detect_transitions` | Pure helper for Sandbox lifecycle subtype detection. |
| `backend/src/sandbox/k8s/pod_watcher.rs::detect_server_transitions` | Pure helper for Preview server status transitions. |

Current trace:

1. `spawn_pod_lifecycle_watcher` starts one watcher task and its leadership loop.
2. `run_pod_watch` receives runtime observations and periodically sweeps remembered state.
3. `process_watcher_event` handles one observed pod change.
4. `extract_pod_observed_state` builds `PodObservedState`.
5. `detect_transitions` and `detect_server_transitions` return raw transition facts.
6. `process_watcher_event` combines those facts with duplicate suppression, intent correlation, details fetching, event publishing, crash auto-stop, server auto-disable, cleanup, and retries.
7. Lifecycle operations are called only for remediation and intent changes.

Observed friction:

| Friction | Effect |
|---|---|
| Policy is split between pure helpers and the async watch body. | Tests protect helper cases, not complete observation plans. |
| Crash classification, duplicate publish suppression, lifecycle correlation, and remediation interact after transition detection. | A small behavior change requires reading most of the watcher implementation. |
| Event publishing and remediation decisions sit beside runtime IO. | The natural test surface is wider than the behavior under change. |
| Server and Sandbox transitions are detected separately but reconciled together. | The code path that forces server failures after Sandbox failures is not represented as one explicit policy result. |

Dead or intentionally unchanged paths:

| Path | Treatment |
|---|---|
| Lifecycle intent writes | Out of scope. Lifecycle remains the only owner. |
| Runtime leader election | Out of scope. It stays in the watcher adapter. |
| Raw event names and user-visible states | Out of scope. This change restructures the implementation. |
| Docker runtime observation | Out of scope for this spec. |

## Planned solution

Create a deep in-process module named `PodObservationPolicy`.
The module's interface is small:
it evaluates already-extracted observed state and returns one `ObservationPlan`.

The watcher remains the runtime adapter.
It receives raw observations, extracts `PodObservedState`, fetches runtime details when the plan asks for them, publishes planned events, and executes planned remediation through lifecycle.

The policy becomes the place where observed-state behavior concentrates:

```rust
pub struct PodObservationPolicy;

impl PodObservationPolicy {
    pub fn evaluate(
        previous: Option<&PodObservedState>,
        current: &PodObservedState,
        context: ObservationContext,
    ) -> ObservationPlan;
}
```

The exact Rust names may change during implementation, but the interface shape should not.
It should remain a pure evaluation step over domain state.

`ObservationPlan` carries the pure decisions the watcher needs before doing IO:

| Plan part | Meaning |
|---|---|
| Sandbox events | User-visible Sandbox state changes and payload data that does not require a late detail fetch. |
| Preview server events | User-visible Preview server state changes. |
| Lifecycle observations | Correlated facts that lifecycle may use to suppress or reclassify observed runtime changes. |
| Duplicate suppression | Whether a high-frequency lifecycle event should skip publishing because the same Sandbox status was already published. |
| Summary flags | Whether the transition includes error, fatal, completed, or first-ready startup behavior. |

The watcher adapter keeps these responsibilities:

| Responsibility | Reason |
|---|---|
| Runtime watch stream | It is IO, not policy. |
| Leader election | It is runtime coordination, not observed-state classification. |
| Raw runtime reads | It couples to runtime client types. |
| Detail fetching | It needs runtime IO and may fail independently. |
| Plan execution | It calls event publishers and lifecycle adapters. |
| Remediation decisions that require IO context | Crash auto-stop, server auto-disable, cleanup, retry, and forced server failure still depend on repositories, lifecycle calls, and runtime clients. |
| Lifecycle calls | Lifecycle remains the owner of intent and runtime operations. |

Expected module shape:

```
backend/src/sandbox/k8s/
├── pod_watcher.rs              # watcher adapter and plan execution
├── pod_observation_policy.rs   # policy interface and implementation
```

The important seam is the policy interface, not the file count.

### Problem

Maintainers changing Sandbox observation behavior currently need to edit a large watcher implementation where runtime IO, event publishing, lifecycle correlation, and remediation decisions are interleaved.

The cost is low locality:
bugs in node reclaim handling, eviction handling, crash auto-stop, server auto-disable, and duplicate publish suppression are fixed in one file but not at one interface.

The cost is also low leverage:
tests can call pure helpers such as transition detection, but the behavior the system depends on is the complete plan for one observation.

### Behaviour

Actor: maintainer changing Sandbox observation behavior.

| Given | When | Then |
|---|---|---|
| A runtime observation has been converted into `PodObservedState`. | The policy evaluates previous state, current state, and context. | It returns one `ObservationPlan` describing Sandbox transitions, Preview server transitions, lifecycle observations, duplicate suppression, and summary flags. |
| A clean Sandbox stop is observed while lifecycle still wants a running Sandbox. | The policy evaluates the observation. | The plan does not cancel the Sandbox and leaves recovery to the runtime adapter. |
| A resource-pressure eviction is observed. | The policy evaluates the observation. | The plan surfaces an error-level Sandbox event with the runtime cause. |
| A node-removal observation is received. | The policy evaluates the observation. | The plan reports recovery rather than a fatal Sandbox failure. |
| A Preview server changes status. | The policy evaluates the observation. | The plan reports the Preview server transition; crash-threshold remediation remains in the watcher adapter. |
| A Sandbox container crosses the crash auto-stop threshold. | The policy evaluates the observation. | The plan reports the Sandbox error transition; lifecycle stop execution remains in the watcher adapter. |
| A failed runtime object has been terminal long enough for cleanup. | The watcher evaluates remembered state during a sweep. | The policy predicate allows cleanup once, while the watcher performs the delete. |

Actor: watcher adapter.

| Given | When | Then |
|---|---|---|
| The policy returns lifecycle observations. | The watcher executes the plan. | The watcher calls lifecycle classification before publishing suppressible observations. |
| The policy returns error and fatal flags. | The watcher executes the plan. | The watcher performs existing lifecycle and runtime remediation around the planned observations. |

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-where-the-policy-seam-sits) | Where the policy seam sits | After state extraction: policy evaluates `PodObservedState`. | Raw runtime objects would couple the policy interface to runtime shape and make extraction errors harder to isolate. |
| [2](#2-what-crosses-the-interface) | What crosses the interface | `ObservationPlan` with transitions, lifecycle observations, duplicate suppression, and summary flags. | Raw event tuples leave the caller reconstructing policy decisions from partial output. |
| [3](#3-what-stays-in-the-watcher-adapter) | What stays in the watcher adapter | Watch stream, leader election, runtime reads, detail fetches, plan execution, and lifecycle calls. | Executing inside the policy would hide a second adapter inside the policy implementation. |

### 1. Where the policy seam sits

The seam sits after observed-state extraction.
`PodObservationPolicy` receives `PodObservedState`, not raw runtime objects.

This makes the policy interface stable around Moatless domain state:
Sandbox status, Preview server state, restart counts, lifecycle generation, failure detail, node cause, and sticky guards.

Keeping extraction outside the policy preserves a clean adapter split.
Runtime object shape remains in the watcher adapter.
Policy tests can build `PodObservedState` directly and avoid runtime fixtures unless the extraction code itself is under test.

Rejected alternative:
let the policy receive raw runtime objects and own extraction.
That gives more code locality inside one module, but it weakens the interface because every policy test learns runtime object shape.
It also mixes extraction bugs with classification bugs.

### 2. What crosses the interface

The policy returns one `ObservationPlan`.
The plan is the complete policy output for one observed state transition or sweep tick.

The plan carries Sandbox transitions, Preview server transitions, lifecycle observations, duplicate suppression, and summary flags.
This keeps the watcher adapter simple:
it executes the plan and handles IO.

Rejected alternative:
return only event payloads.
That leaves crash auto-stop, server auto-disable, and cleanup decisions in the watcher.
The implemented seam accepts that tradeoff because those decisions still need repositories, lifecycle calls, and runtime clients; moving them now would make the pure policy carry adapter concerns.

### 3. What stays in the watcher adapter

The watcher adapter keeps watch stream ownership, leader election, raw runtime reads, detail fetches, plan execution, and lifecycle calls.

This preserves one adapter around runtime IO.
The policy is pure and testable through its interface.
The watcher remains responsible for fallible IO and side effects.

Rejected alternative:
let the policy execute the plan.
That creates a hidden adapter inside the policy and forces the policy to accept event publishers, lifecycle handles, and runtime clients.
The interface would get wider while the implementation becomes harder to test.

## Seams and testing

External seam:

| Module | Interface | Depth |
|---|---|---|
| `PodObservationPolicy` | `evaluate(previous, current, context) -> ObservationPlan` | High: many observation behaviors behind one pure interface. |

Internal seams:

| Module | Interface | Purpose |
|---|---|---|
| Watcher adapter | Execute `ObservationPlan`. | Runtime IO and side effects. |
| State extraction | Raw runtime object to `PodObservedState`. | Runtime-specific parsing and normalization. |
| Lifecycle | Existing lifecycle operations and observation classification. | Intent and runtime operations remain owned by lifecycle. |

Testing plan:

| Test group | Surface | Cases |
|---|---|---|
| Policy unit tests | `PodObservationPolicy::evaluate` | Node reclaim, resource eviction, clean stop, fatal crash, duplicate publish suppression, server failure forcing, crash auto-stop threshold, failed-object cleanup. |
| Watcher adapter tests | Plan execution helper | Detail-fetch request handling, lifecycle observation calls, remediation command dispatch. |
| Extraction tests | `extract_pod_observed_state` | Runtime object parsing and normalization only. |
| Regression tests | Existing watcher tests migrated to policy tests | Current false-positive crash and eviction cases must survive. |

The interface is the test surface.
Do not keep testing policy by reaching through `process_watcher_event` unless the test needs adapter IO or side effects.

The deletion test passes:
deleting `PodObservationPolicy` would move node reclaim, eviction, intentional stop, duplicate suppression, server failure, lifecycle correlation, and remediation decisions back into the watch loop.
That complexity is exactly what the module earns locality over.

## Out of scope

| Area | Reason |
|---|---|
| Lifecycle intent ownership | Existing lifecycle architecture says lifecycle owns intent and runtime operations. |
| Runtime leader election | It is adapter mechanics, not policy. |
| User-visible state names | This is an implementation architecture change. |
| Docker runtime observation | The candidate came from the large Kubernetes watcher file. |
| Broad Sandbox lifecycle redesign | This spec deepens one watcher policy seam. |

## Open questions

None.

## Ticket breakdown

None.
