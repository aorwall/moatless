---
name: adapter-development
description: Build or change an Adapter in backend/src/adapters — canonical module shape, durable-delivery guardrails, and seam tests. Use when adding an Adapter, changing inbound webhook handling, delivery kinds, outbound replies, reactions, or binding status, or when migrating a flat Adapter (Slack, Telegram, Linear, Webhook) toward the shape GitHub PR already has.
---

# Adapter Development

Backend only: everything under `backend/src/adapters/`. A new `AdapterKind` also needs
`crates/api-schemas`, the generated API client, and frontend work — out of scope here; name it as
follow-up work instead of half-wiring it.

Vocabulary is `CONTEXT.md`. Use **Adapter**, **Adapter connection**, **Adapter delivery**,
**Binding**, **Subscription**, **Sender**, **Task** exactly as defined there. "Provider" is never a
standalone term.

## Route

| You are | Go to |
|---|---|
| Adding an Adapter | [Add an Adapter](#add-an-adapter), with [references/new-adapter-wiring.md](references/new-adapter-wiring.md) |
| Changing an existing Adapter | [Change an Adapter](#change-an-adapter) |
| Either | [Canonical shape](#canonical-shape), [Invariants](#invariants), [Prove it](#prove-it), [Verify](#verify) |

## Canonical shape

`backend/src/adapters/providers/github_pr/` is the worked example and the only Adapter that
currently matches. Slack, Telegram, Linear, and Webhook are mid-migration;
`.plans/remaining-adapter-breakup.md` holds the staging and the known gaps. Read the equivalent
`github_pr` module before writing a new one.

| Module | Owns | Keeps out |
|---|---|---|
| `adapter.rs` | The Adapter type, construction, `Adapter` identity, static content limits, `pub(super)` accessors for siblings. | Workflow behavior, and any method that exists only to forward to a sibling module. |
| `handlers.rs` | HTTP extraction, metrics, response mapping. | Policy, database reads, anything but mapping the ingress result. |
| `ingress.rs` | `receive(state, adapter, Request) -> Result<Outcome, IngressError>`: verification, connection and Subscription resolution, filtering, normalization, effect application, delivery planning, admission, outcome classification. | Raw payload parsing, workflow execution. |
| `webhook.rs` | `normalize(event, context) -> Normalization`: event parsing, message construction, audience assignment, external ids, ignore reasons, workflow candidates. | Database writes, Sender API calls, admission, routing. Sender-to-user lookup is the one allowed read. |
| `delivery.rs` | Kind constants, payload types, handler registration, current-state reload, permanent-versus-retryable classification. | Workflow logic — dispatch to the owning module. |
| `routing.rs` | Binding assembly, provenance, Task creation, shared audience-router invocation, cursor updates. | Raw payload parsing, credential selection. |
| One module per durable kind | Candidate derivation and execution for that kind. | Any dependency on `ingress.rs`. |
| `auth.rs` | Identity resolution and authenticated client acquisition, named by use case (inbound sender, outbound reply, reaction, status refresh, durable work). | A generic `resolve_token(user)` that leaves user selection to every caller. |
| `subject.rs` | Fallible subject parse from Binding or thread data, canonical key, destination construction. | String parsing duplicated into callers. |
| `outbound.rs`, `reactions.rs`, `status.rs` | Their capability traits, implemented directly for the Adapter type. | Delegation through `adapter.rs`. |
| `api.rs` | Concrete Sender API calls, with a `with_base_url` constructor for tests. | A trait mirroring every endpoint. |
| `text.rs`, `external_id.rs`, `config.rs`, `signature.rs` | Rendering, id codecs, Subscription config parsing, signature verification. | Behavior that belongs to a workflow. |

Dependency direction:

```text
handlers → ingress → webhook (normalize)
                   → delivery → workflow modules → bindings::runtime / subject_events
capabilities (outbound, reactions, status) → auth → api
```

Capability traits live in `backend/src/adapters/capabilities.rs`. Implement only the ones the
Adapter supports and register exactly those in `registry::Adapters::register_<name>`; an
unregistered capability is a clean "unsupported", a registered-but-rejecting one is a runtime
surprise.

## Change an Adapter

The target shape is normative even where the Adapter has not reached it yet.

1. Name the canonical module for the behavior you are changing, from the table above.
2. Working there already? Make the change.
3. Not there yet? Pin the behavior first: write a test that fails if the move alters anything
   observable, and get it green on the current shape.
4. Move that behavior — and only that behavior — into a module of the canonical name, creating it
   if needed. Everything you are not changing stays where it is.
5. Make your change in the new module.
6. Land the move and the change as separate commits: `refactor:` then `feat:` / `fix:`.

Blast radius is one workflow. When the move pulls in a second workflow, stop, make the change where
the code is, and add the missing extraction to the staging in
`.plans/remaining-adapter-breakup.md`.

Done when the behavior you changed lives in its canonical module, a test exercises it at that
module's interface, `adapter.rs` gained no forwarding method, and the characterization test from
step 3 still passes unchanged.

## Add an Adapter

1. Decide which capabilities the Adapter supports, from `capabilities.rs`. This decides the module
   set and the registrar.
2. Decide the durable delivery kinds — one per independent effect one Sender delivery can produce.
   The kind string is permanent; see [Invariants](#invariants).
3. Decide the Sender's own delivery identity, and the fallback when the Sender sends none. This
   becomes the deduplication key.
4. Build the module tree from [Canonical shape](#canonical-shape), starting with `subject.rs`,
   `auth.rs`, and `api.rs`, then `webhook.rs`, then `ingress.rs`, then the workflow modules.
5. Wire it: follow [references/new-adapter-wiring.md](references/new-adapter-wiring.md) — eleven
   touchpoints, each with its file.
6. Keep the module private: `pub(super)` inside the Adapter, and `mod.rs` exporting only what the
   route composition root, the registry, and the delivery registration need.

Done when a signed request to the new route admits a durable record, the worker executes it, the
registry reports exactly the intended capabilities, and no test imports an internal of the Adapter.

## Invariants

These break production when violated. Each is stated as the behavior to keep.

**Acknowledge only what is durable.** Ingress admits before it reports success; an admission failure
reaches the Sender as a failure so it redelivers. Ingress rejections that redelivery cannot repair —
malformed body, bad signature — stay acknowledged, logged, and metered. A delivery whose work cannot
be given an identity is refused with `400`. See `adapters/inbound_delivery.rs:192` and
`github_pr/ingress.rs:63`.

**Kind strings are permanent, and one kind is one effect.** Deduplication and the retry budget are
per `(adapter, kind, sender delivery id)`. Folding two effects into one kind lets a permanent failure
in one spend the other's attempts. One Sender delivery may fan into several Adapter deliveries;
nothing may depend on the order they execute in. See `linear/delivery.rs:26` and CONTEXT.md.

**Delivery payload fields are add-only, and every added field carries `#[serde(default)]`.** During a
rolling deploy the new build executes records the old build admitted; a payload that stops
deserializing is work that goes terminal. See `adapters/inbound_delivery.rs:39`.

**Payloads carry ids, not snapshots.** Store the connection and Subscription ids and re-read them at
execution time, so the work runs against current configuration and credentials never sit in a table
retained for weeks. See `github_pr/delivery.rs:130`.

**A deleted Adapter connection fails the record permanently. A deleted Subscription does not.**
Existing Bindings keep receiving events; only new trigger-created Task behavior is gated on the
Subscription. See `github_pr/ingress.rs:130`.

**Normalization returns values.** Sender-to-user lookup is the only I/O allowed inside it. An effect
discovered during normalization — a status projection, a notice to post, a workflow candidate — is
returned and applied by ingress or by a durable handler. Moving sender lookup later would change the
serialized `InboundMessage.user_id` a retry observes.

**External id formats are persisted.** They participate in echo filtering and deduplication, so
extend them through `external_id_aliases` rather than changing an existing format.

**Audience belongs to the shared router.** Adapters assemble facts and hand them to
`adapters/subject_events.rs`; recipient logic is not reimplemented per Adapter.

**Best-effort side effects stay off the delivery seam.** Outbound activity notifications and usage
timestamps are allowed to be lost; admitting them makes every webhook slower and the table larger
for no correctness gain.

**`ScopeContext::system()` is lint-disallowed.** A delivery handler runs with no authenticated user
and legitimately needs it — annotate each call with `#[allow(clippy::disallowed_methods)]` and a
`// reason:` comment naming why the bypass is safe there. See `clippy.toml`.

## Prove it

| Seam | Technique |
|---|---|
| `normalize` | Table-driven, in-process, no network. Cover every event and action, each ignore reason, audience, external ids, and workflow candidates. |
| `receive` | Local database plus a stubbed Sender API. Cover signature policy, resolution, ignore reasons and metric labels, the admission identity, duplicate admission, and the acknowledgment class of each error variant. |
| Registered delivery handler | Payload round trip, restart recovery, current-state reload, missing-connection permanent failure, transient-failure retry. |
| Capability implementations | Stubbed Sender API. Assert request count and order where order affects rate limits. |
| `auth.rs` | Every identity precedence rule, plus absent and empty credentials. |

Stub the Sender by injecting a base URL through the API client's constructor —
`SlackApi::with_base_url` and `TelegramApi::with_base_url` are the pattern. Shared helpers live in
`backend/src/adapters/test_support.rs`: `stub_provider`, `UNREACHABLE_BASE`, `transport_error`,
`source_chain`, `response_snapshot`.

Expect the first characterization test on a flat Adapter to be blocked: `SlackAdapter::new`,
`TelegramAdapter::new`, and `LinearAdapter`'s API builders each construct their client internally,
so there is nowhere to inject the stub even though the clients support it. Add the injection point
before the test — a constructor that takes an already-built client — and give a new Adapter one from
the start. A test that reaches the real Sender is not a test.

Assert the API-call budget of any workflow you touch. A refactor that adds a call to unchanged
behavior is a regression even when every test passes.

## Verify

```bash
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend
task test:integration          # when anything under backend/tests/ changed
```

`task test:integration` needs `DATABASE_URL`. Every file in `backend/tests/` except
`supervisor_test.rs` is behind `#![cfg(feature = "db-tests")]`, so a plain `cargo test` passes green
having compiled almost none of them — and CI runs that same plain line.

Test-only code is not linted by the CI clippy step, which has no `--all-targets`. Run
`cargo clippy -p moatless-vibe-backend --all-targets` yourself before claiming a test helper is
clean.
