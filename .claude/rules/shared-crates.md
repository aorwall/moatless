---
paths:
  - "crates/**/*.rs"
---

# Shared Crate Conventions

## Events Crate (`crates/events/`)
- Enums use `#[serde(rename_all = "...")]`: `"camelCase"` for `EventType`, `"snake_case"` for status/subtype enums
- Every enum must implement: `Display`, `TryFrom<&str>`, `as_str() -> &'static str`
- Serialized values MUST match what the frontend expects — changing them is a breaking change

## Sandbox Proto (`crates/sandbox-proto/`)
- Define `From` and `TryFrom` conversions between protobuf-generated types and domain types
- Never use proto types directly in business logic — convert at the boundary

## Claude Agent SDK (`crates/claude-agent-sdk/`)
- `Arc<dyn Trait>` for testability (transport, stdin writer)
- Bounded `mpsc` channels (256 capacity) for backpressure
- Custom deserialization via `SdkMessage::from_value()` — not `#[serde(tag)]`

## Feature Flags
- `#[cfg(feature = "openapi")]` for utoipa derives needed only by backend
