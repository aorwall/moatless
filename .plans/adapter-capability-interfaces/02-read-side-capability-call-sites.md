# Ticket 02: Migrate read-side adapter call sites to capabilities

## Goal

Move read-side adapter behavior off the broad `Adapter` trait and onto capability-specific registry lookups.

## Dependency

Start after Ticket 01 has merged or use Ticket 01's PR branch as the base branch.

## Spec

Use `.plans/adapter-capability-interfaces.md` on branch `adapter-capability-interfaces`.

Implement phase 3 from the spec.

## Scope

- Migrate status and presentation call sites to capability lookups:
  - `bindings::status_service`
  - task connection read handlers
  - `bindings::runtime`
  - loop firing/listen code that only builds destinations from existing subjects
- Preserve current fallback behavior:
  - no live status falls back to persisted status
  - no context fetch returns no extra context
  - labels, aliases, reply instructions, thread hints, and prefixes match existing output
- Add or update focused tests around status fallback and inbound presentation behavior where coverage exists.

## Non-Goals

- Do not migrate outbound send/reply/reaction call sites.
- Do not remove the broad `Adapter` trait.
- Do not split provider implementation modules yet.

## Acceptance

- No read-side call site uses the broad `Adapter` trait when a narrower capability exists.
- Existing behavior for Slack, Telegram, GitHub PR, Linear, and Webhook is preserved.
- `cargo check -p moatless-vibe-backend` passes.
- Run focused tests for binding status/runtime/task connection behavior and report exactly what ran.

