# Wiring an adapter

Read this when adding a provider or changing its capabilities. Trace an existing
provider through these owners; do not put provider construction in the shared registry.

1. Add domain vocabulary in [types](../../../../crates/types/src/adapter.rs).
   Follow [where a type belongs](../../rust-dev-guidelines/references/file-roles.md#choose-a-home-for-a-deduplicated-shared-type)
   for wire DTOs, persistence, and ports. Regenerate the API client when the public schema changes,
   and implement the corresponding frontend behavior within the feature scope.
2. Implement Sender-specific code in a `crates/adapter-*` crate. Export only the
   constructors, capability registration, routes, and delivery handlers composition needs.
3. Implement supported traits from
   [capabilities.rs](../../../../crates/adapter-model/src/capabilities.rs).
   Register exactly those capabilities through the provider's `register_into`.
   [registry.rs](../../../../crates/adapter-model/src/registry.rs) stores trait objects.
4. Register durable kinds and typed payloads through the adapter-model delivery
   contracts. Execution belongs to provider workflows; the worker and store belong
   to [loops delivery](../../../../crates/loops/src/adapters/delivery/mod.rs).
5. Wire concrete installations, dependencies, and delivery registration in
   [adapter_installations.rs](../../../../crates/server/src/adapter_installations.rs).
   Follow the facades in `crates/server/src/adapters/providers` and mount routes in
   [routes.rs](../../../../crates/server/src/adapters/routes.rs).
6. Add required application configuration through
   [core config](../../../../crates/core/src/config.rs) and deployment configuration.
   Reuse existing credential ownership and connection services.
7. Extend [registry matrix tests](../../../../crates/server/src/adapters/registry_matrix_tests.rs)
   and provider seam tests. Prove signed ingress admits work, duplicate delivery
   does not duplicate the effect, the worker executes it, and unsupported capabilities
   are absent from the registry.

Use the invariants in [the skill](../SKILL.md) as the behavior checklist. No test
should need a real Sender credential or import a private provider module.
