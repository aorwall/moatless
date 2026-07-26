# Shared Rust Crates

Apply this reference to Rust changes under `crates/`.

## Wire Contracts

- Serialized event strings and protobuf discriminants are wire contracts. Any
  change updates every conversion/serialization surface and exhaustive
  round-trip tests.
- Prost enum fields are open integer inputs at runtime. Validate unknown values
  at the boundary instead of silently mapping them to a default variant.
- Prefer a narrower transport enum when the domain enum contains values the
  protocol cannot represent.

## Dependency and Runtime Boundaries

- Keep `utoipa` and OpenAPI derives feature-gated so shared crates do not force
  API-only dependencies on non-API consumers.
- Preserve SDK behavior across direct subprocess and terminal-backed execution
  paths when changing transport, input, cancellation, or shutdown.
- Use bounded channels where memory must be bounded, but choose capacity from
  the producer/consumer and overload contract rather than a repository-wide
  magic number.
- Introduce trait-object seams only when production variants or exercised test
  doubles justify the interface.
