# Ticket 04: Split provider implementations by capability and retire broad defaults

## Goal

Break provider adapter implementations into capability-specific implementation blocks or modules, then remove broad `Adapter` defaults that no call site uses.

## Dependency

Start after Tickets 02 and 03 have merged.

## Spec

Use `.plans/adapter-capability-interfaces.md` on branch `adapter-capability-interfaces`.

Implement phases 6, 7, and 8 from the spec.

## Scope

- Split provider implementation code by capability trait:
  - Slack
  - Telegram
  - GitHub PR
  - Linear
  - Webhook
- Keep capability methods near provider helpers they use.
- Remove or shrink the old broad `Adapter` trait once call sites no longer need it.
- Delete broad default methods that have been replaced by missing-capability registry behavior.
- Keep typed provider handles for concrete-only provider workflows.
- Add compile-time or unit-test coverage that would fail if a provider accidentally registers an unsupported capability.

## Non-Goals

- Do not change adapter behavior.
- Do not rename provider concepts beyond what is needed for capability layout.
- Do not remove typed provider handles.

## Acceptance

- Provider files no longer have one large broad-trait implementation that mixes unrelated capabilities.
- The old broad trait is removed or reduced to identity-only compatibility, whichever is the smallest safe end state.
- Unsupported behavior comes from missing capability registration, not inherited broad defaults.
- `cargo check -p moatless-vibe-backend` passes.
- `cargo clippy -p moatless-vibe-backend -- -D warnings` passes.
- `cargo test -p moatless-vibe-backend` passes, unless there is an environment failure that is clearly reported.

