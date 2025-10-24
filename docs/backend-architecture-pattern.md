# Backend Feature Architecture Pattern

This document defines the target architecture for `apps/backend`. It is the reference pattern to follow as we migrate from the current service-centric layout toward feature-scoped vertical slices.

## High-Level Goals

- Organise code by feature first, technology second.
- Provide clear seams between HTTP contracts, business logic, storage, and external integrations.
- Co-locate related assets (routes, services, tests, utilities) to make ownership obvious.
- Reduce shared mutable state to a thin `shared/` layer with stable, reusable primitives.
- Rely on the consolidated `packages/api-contract` module for OpenAPI specs and schema validation shared with the frontend.

## Target Directory Layout

```
apps/backend/src/
  app.ts
  shared/
    http/
      middleware/
      hono-helpers.ts
    storage/
      storage-backend.ts
      storage.types.ts
    messaging/
    telemetry/
    utils/
      date.utils.ts
      logger.ts
    config/
      env.ts
  features/
    <feature-name>/
      api/
        <domain>.routes.ts
        __tests__/
      services/
        <domain>.service.ts
      infrastructure/
        <dependency>.client.ts
        <dependency>.storage.ts
      types/
        <domain>.types.ts
      utils/
        <domain>.utils.ts
      __tests__/
```

### Feature Slice Responsibilities

| Directory | Purpose | Naming conventions |
| --- | --- | --- |
| `api/` | Route registration, HTTP handlers, request/response mapping. Depends on services only. | `<domain>.routes.ts`, supporting files named `*.controller.ts` if needed. |
| `services/` | Business logic and application orchestration. Stateless where possible. | `<domain>.service.ts` per bounded context. |
| `infrastructure/` | Feature-owned data access, external SDK clients, adapters. | `<name>.client.ts`, `<name>.storage.ts`. Implements interfaces defined in services/types. |
| `types/` | Domain-specific types/value objects not exposed through the public API. | `<domain>.types.ts`. |
| `utils/` | Feature-scoped helpers. Favor pure functions. | `<name>.utils.ts`. |

Feature directories can expose a single entry point (e.g. `index.ts`) that wires infrastructure implementations into services and exports `register<Feature>Routes` for `app.ts` to consume.

### API Contracts & Schemas

- Define routing contracts and schemas inside `packages/api-contract`.
- Location pattern:
  - `packages/api-contract/src/<feature>/<domain>.contract.ts` – exports Hono OpenAPI route definitions.
  - `packages/api-contract/src/<feature>/<domain>.schemas.ts` – exports zod schemas for request/response payloads used by both backend and frontend.
- Backend `api/<domain>.routes.ts` consumes the contract definitions; frontends import the same schemas for client validation.

### Services & Types

- Each `.service.ts` exposes the use-case surface through explicit interfaces. Keep side effects in infrastructure files.
- Domain-specific types that are not part of the HTTP contract live in `types/<domain>.types.ts`. These are imported by services and infrastructure but never by consumers outside the feature.

### Infrastructure & Storage

- Shared storage primitives (generic `get`, `set`, append, etc.) live in `shared/storage/storage-backend.ts`.
- Each feature implements its own storage adapter in `infrastructure/<name>.storage.ts`, composed from the shared backend. No monolithic storage module.
- External SDK or HTTP integrations belong in `<name>.client.ts`. Wrap third-party interactions behind interfaces defined in services to keep test seams clean.

### Shared Layer (`shared/`)

- Houses cross-cutting utilities that have no feature ownership: logging, telemetry bootstrap, middleware, primitive storage interfaces, shared configuration helpers.
- Avoid leaking feature logic into `shared/`. If a utility starts to depend on feature-specific concepts, move it under that feature instead.

### Testing Guidance

- Colocate feature tests under the corresponding directories (`api/__tests__`, `__tests__` at feature root) to maintain boundaries.
- Shared test utilities that are reused across slices can live in `shared/test-utils/`.

### Migration Playbook

1. Identify a feature slice to migrate (e.g. `uploads`).
2. Create the `features/<feature-name>/` directory with the sub-structure above.
3. Move the route registration into `api/<domain>.routes.ts`, referencing contracts from `packages/api-contract`.
4. Relocate business logic into `services/<domain>.service.ts`. Expose a small facade for the rest of the app.
5. Split storage/clients into `infrastructure/*.storage.ts` and `infrastructure/*.client.ts`, backed by `shared/storage` primitives.
6. Update `app.ts` (or the service container) to instantiate the feature through its exported factory.
7. Delete the legacy code after verifying tests.

### Additional Notes

- Keep dependency direction from `api -> services -> infrastructure`. `types` and `utils` can be imported where needed but should not import infrastructure.
- Prefer dependency injection (constructor parameters or factory functions) when wiring services to storage/clients, easing testability.
- Document each feature with a short `README.md` outlining its public surface and dependencies once migrated.

This pattern provides a consistent blueprint for new features and a north star for ongoing refactors.
