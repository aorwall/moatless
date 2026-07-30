# Ticket 03: Migrate outbound and reaction adapter call sites to capabilities

## Goal

Move send, reply, direct-message, post-binding follow-up, and reaction behavior off the broad `Adapter` trait and onto capability-specific registry lookups.

## Dependency

Start after Ticket 01 has merged or use Ticket 01's PR branch as the base branch.

This ticket may run in parallel with Ticket 02 if both are based on Ticket 01, but expect merge conflicts in shared adapter registry imports.

## Spec

Use `.plans/adapter-capability-interfaces.md` on branch `adapter-capability-interfaces`.

Implement phases 4 and 5 from the spec.

## Scope

- Migrate outbound call sites:
  - `bindings::reply`
  - `bindings::outbound`
  - adapter direct-send HTTP handlers
  - Loop-created new-thread send paths
  - post-Binding follow-up hooks
- Migrate reaction call sites:
  - add reaction
  - remove reaction
- Preserve current unsupported-operation mapping for providers that do not implement the capability.
- Preserve per-message reply fallback behavior where it exists today.
- Add focused tests for missing outbound/reaction capability behavior where coverage exists.

## Non-Goals

- Do not migrate read-only status/presentation call sites unless required by compilation.
- Do not remove the broad `Adapter` trait.
- Do not split provider implementation modules yet.

## Acceptance

- Outbound/reaction call sites ask the registry for outbound/reaction capabilities instead of depending on the broad adapter object.
- Existing Slack, Telegram, GitHub PR, Linear, and Webhook send/reaction behavior is preserved.
- `cargo check -p moatless-vibe-backend` passes.
- Run focused reply/outbound/reaction tests and report exactly what ran.

