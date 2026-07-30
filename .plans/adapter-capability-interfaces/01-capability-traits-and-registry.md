# Ticket 01: Add adapter capability traits and registry slots

## Goal

Introduce the adapter capability interfaces and make the adapter registry expose capability lookups while preserving the current broad `Adapter` trait and existing behavior.

## Spec

Use `.plans/adapter-capability-interfaces.md` on branch `adapter-capability-interfaces`.

Implement phases 1 and 2 from the spec.

## Scope

- Add capability traits in `backend/src/adapters/mod.rs` or a nearby module:
  - identity
  - user identity
  - destination builder
  - outbound
  - reactions
  - inbound presentation
  - binding status
- Keep the current broad `Adapter` trait available during this ticket.
- Register the existing adapters into optional capability slots.
- Keep typed provider handles for GitHub PR, Linear, Slack, and Telegram.
- Add registry methods that return the requested capability or the existing unsupported behavior.
- Add focused registry tests that prove expected capability presence and absence for Slack, Telegram, GitHub PR, Linear, and Webhook.

## Non-Goals

- Do not migrate call sites off the broad trait yet except where needed to compile the registry.
- Do not split provider implementation modules yet.
- Do not change user-visible adapter behavior.

## Acceptance

- `cargo check -p moatless-vibe-backend` passes.
- `cargo test -p moatless-vibe-backend adapters` or the nearest focused adapter registry test target passes.
- Full backend tests are not required if the focused suite and check pass, but note if they were skipped.
- A PR is opened with a summary of the new capability registry seam and any intentionally deferred broad-trait usage.

