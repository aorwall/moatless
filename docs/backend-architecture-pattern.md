# Backend Feature Architecture Pattern

This document defines the target architecture for `apps/backend`. It is the reference pattern to follow as we migrate from the current service-centric layout toward feature-scoped vertical slices.

## High-Level Goals

- Organise code by feature first, technology second.
- Provide clear seams between HTTP contracts, business logic, storage, and external integrations.
- Co-locate related assets (routes, services, tests, utilities) to make ownership obvious.
- Reduce shared mutable state to thin `shared/` and `infrastructure/` layers with reusable primitives (including `infrastructure/clients` and `infrastructure/storage`).
- Rely on the consolidated `packages/api-contract` module for OpenAPI specs and validation schemas shared with the frontend.

## Target Directory Layout

```
apps/backend/src/
  app.ts
  shared/
    http/
      middleware/
      hono-helpers.ts
    utils/
      date.utils.ts
      logger.ts
    config/
      env.ts
  infrastructure/clients/
    github.client.ts
    auth0.client.ts
    netlify.client.ts
  infrastructure/storage/
    storage-backend.ts
    file-storage.ts
    storage.types.ts
  features/
    <feature-name>/
      api/
        <domain>.routes.ts
        __tests__/
      services/
        <domain>.service.ts
      storage/
        <domain>.storage.ts
      types/
        <domain>.types.ts
      utils/
        <domain>.utils.ts
      index.ts
      __tests__/
```

### Feature Slice Responsibilities

| Directory | Purpose | Naming conventions |
| --- | --- | --- |
| `api/` | Route registration, HTTP handlers, request/response mapping. Depends on services only. | `<domain>.routes.ts`, optional helpers as `*.controller.ts`. |
| `services/` | Business logic and application orchestration. Stateless where possible. | `<domain>.service.ts` per bounded context; exported via feature index. |
| `types/` | Domain-specific types/value objects not exposed via HTTP contracts. | `<domain>.types.ts`. |
| `utils/` | Feature-scoped utilities and helpers. | `<name>.utils.ts`. |

Feature directories expose an `index.ts` that wires feature services to the shared infrastructure (clients + storage primitives) and exports `register<Feature>Routes` along with any public facades.

### API Contracts & Schemas

- Define routing contracts and schemas inside `packages/api-contract`.
- Location pattern:
  - `packages/api-contract/src/<feature>/<domain>.contract.ts` – exports Hono OpenAPI route definitions.
  - `packages/api-contract/src/<feature>/<domain>.schemas.ts` – exports zod schemas for request/response payloads used by both backend and frontend.
- Backend `api/<domain>.routes.ts` consumes the contract definitions; frontends import the same schemas for client validation.

### Services & Internal Types

- Each `.service.ts` exposes the use-case surface through explicit interfaces. Keep side effects out of services by delegating to injected storage/client dependencies.
- Domain-specific types that are not part of the HTTP contract live in `types/<domain>.types.ts`. These are imported by services, utils, and storage adapters but never by other features directly.

### Shared Infrastructure Layer

- `apps/backend/src/infrastructure/` houses reusable clients and adapters (e.g. GitHub, Auth0, Netlify, event buses, telemetry exporters).
- Clients follow the `*.client.ts` convention and implement interfaces defined by consuming features.
- Features obtain instances via dependency injection in their `index.ts` (or the global service container) rather than instantiating clients directly.

### Storage Layer

- Feature storage adapters live inside each feature directory (e.g. `features/<feature>/storage/`) and reuse the base primitives from `infrastructure/storage/`.
- The base storage modules under `infrastructure/storage/` expose generic primitives (`get`, `set`, `append`, etc.).
- Feature storage adapters live inside each feature directory (e.g. `features/<feature>/storage/<name>.storage.ts`) and encapsulate data access for that feature only.

### Shared Layer (`shared/`)

- Hosts cross-cutting utilities that have no feature ownership: middleware, logger setup, common config helpers, pure utilities.
- Shared middleware should live under `shared/http/middleware/` and be re-exported by features as needed.
- If a utility starts depending on feature-specific concepts, move it under that feature instead.

### Testing Guidance

- Colocate feature tests under the corresponding directories (`api/__tests__`, `__tests__` at feature root) to maintain boundaries.
- Shared test utilities that are reused across slices can live in `shared/test-utils/`.

### Migration Playbook

1. Identify a feature slice to migrate (e.g. `uploads`).
2. Create the `features/<feature-name>/` directory with the structure above.
3. Move route registration into `api/<domain>.routes.ts`, referencing contracts from `packages/api-contract`.
4. Relocate business logic into `services/<domain>.service.ts`, exposing a small facade via the feature `index.ts`.
5. Extract reusable clients into `infrastructure/clients/`, move shared storage primitives into `infrastructure/storage/`, and relocate feature-specific persistence into `features/<feature>/storage/`, wired through constructor injection.
6. Update `app.ts` (or the DI container) to instantiate the feature through its exported factory and to register shared middleware from `shared/http/middleware/`.
7. Delete the legacy code after verifying tests.

### Additional Notes

- Keep dependency direction `api -> services -> (storage/infrastructure)`; avoid inward references from shared layers to features.
- Prefer dependency injection (constructor parameters or factory functions) when wiring services to storage/clients, easing testability.
- Document each feature with a short `README.md` outlining its public surface and dependencies once migrated.

This pattern provides a consistent blueprint for new features and a north star for ongoing refactors.
