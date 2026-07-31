# New Adapter wiring — backend touchpoints

Every place a new Adapter must appear for the backend to compile, route, register, and execute.
Line numbers are anchors for finding the block, not exact addresses. Work top to bottom: each step
compiles once the ones above it are done.

No database migration is needed. `integration_connections.adapter_kind` is a `text` column with no
constraint on its values (`backend/migrations/0051_integrations_triggers_bindings.sql:8`), and the
`adapter_delivery` table is keyed by `(adapter, kind, event_id)` strings.

## 1. `crates/api-schemas/src/enums.rs` (~830)

Add the variant to `AdapterKind`, plus its arm in `as_str` and in `parse`. The string is the
persisted `adapter_kind` value and the `provider_key` — permanent once anything is stored.

`AdapterKind::parse` falls back to `Webhook` for unknown strings in
`backend/src/adapters/common.rs:32`, so a missing `parse` arm degrades silently instead of failing.
Add both arms in the same edit.

This crate also feeds the generated API client and the frontend. Those are out of scope for the
backend change; name them as follow-up work.

## 2. `backend/src/config.rs`

Three blocks:

- The `XConfig` struct (~719 for `SlackConfig`). Implement `Debug` by hand and redact every secret,
  matching the neighbours.
- The field on `Config` (~921).
- The population in `Config::from_env` (~943, Slack at ~1151) and in `Config::test_minimal`
  (~1336, Slack at ~1474). Missing the second breaks every test that builds a config.

## 3. `backend/src/adapters/providers/<name>/`

The module tree from the canonical shape. Start with `mod.rs`, `adapter.rs`, `subject.rs`,
`auth.rs`, `api.rs`.

## 4. `backend/src/adapters/providers/mod.rs`

Declare the module.

## 5. `backend/src/adapters/mod.rs` (~24)

Add the module to `pub use providers::{...}` only if code outside `providers/` names it.

## 6. `backend/src/adapters/registry/mod.rs`

- A `HashMap<AdapterKind, Arc<dyn …>>` entry already exists per capability; nothing to add there.
- A typed handle field and its `new()` initializer, if the Adapter's own modules need concrete-only
  methods (`slack`, `telegram`, `linear`, `github_pr` have one; `webhook` does not).
- `register_<name>`, calling exactly the `register_*` helpers for the capabilities the Adapter
  implements, and setting the typed handle. The trait-object insert is private precisely so the map
  and the handle cannot disagree — go through the registrar.
- The typed accessor (~232), at the narrowest visibility that compiles.

## 7. `backend/src/adapters/registry/bootstrap.rs` (~20)

Construct the Adapter from config in `Adapters::from_config` and call the registrar. Decide whether
it is always available (Webhook, GitHub PR) or gated on configured credentials (Slack, Telegram).
Log one line either way — an Adapter that silently fails to register is diagnosed by its absence.

## 8. `backend/src/adapters/routes.rs`

Two places, both required for the route to exist and be documented:

- The handler in the `paths(...)` list of `AdapterApiDoc` (~33).
- `.routes(routes!(...))` in `adapter_router` (~72).

Add a legacy `/api/v1/channels/...` alias in `legacy_webhook_aliases` (~96) only when an existing
Sender is already configured against that path.

## 9. `backend/src/adapters/delivery/mod.rs` (~133)

Add the Adapter's `delivery::register` call to `register_builtin_delivery_handlers`. A build without
it leaves admitted records for a replica that has it, rather than failing them — so the omission is
invisible until records pile up.

## 10. `backend/src/adapters/registry/tests.rs`

Assert the registry reports exactly the capabilities the Adapter implements, and no others.

## 11. `backend/tests/`

A `#![cfg(feature = "db-tests")]` test that admits a delivery the way the handler admits it and runs
the worker against it. `adapter_inbound_delivery_test.rs` is the pattern for a chat-shaped Adapter;
`github_webhook_redelivery_test.rs` is the pattern for driving `adapter_router` with real requests.
Prefer the second: it proves the route, the signature policy, and the admission together, and it
imports no Adapter internals.
