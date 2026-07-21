---
paths:
  - "sandbox/src/**/*.rs"
---

# Sandbox Crate Conventions

## Crate Structure
- Dual target: `[lib]` (moatless_sandbox) for integration tests + `[[bin]]` (sandbox) for runtime
- `lib.rs` re-exports all modules — add new modules there too

## State Management
- Use `RwLock<Inner>` pattern for mutable state (see `AgentState`)
- `read().await` for concurrent reads, `write().await` for mutations

## Agent Lifecycle
- Spawn with `tokio::spawn` — always handle errors in both success and failure paths
- Set agent state on completion (`set_idle`) AND on error (`set_error`)
- Check `CancellationToken::is_cancelled()` in processing loops

## gRPC Communication
- Use `sandbox-proto` crate types for all gRPC messages
- `From`/`TryFrom` conversions between proto types and domain types
- Never use proto-generated types directly in business logic

## Event Publishing
- Fire-and-forget via `GrpcEventPublisher`
- Log failures with `tracing::warn!`, never propagate event publish errors

## Configuration
- `Config::from_env()` pattern
- Required vars: `expect("VAR_NAME is required")` — fail fast at startup
- Optional vars: return `Option<String>`

