# Auth & Session Migration Plan

Roadmap for moving authentication, OAuth providers, and session handling into the feature-first architecture described in `docs/backend-architecture-pattern.md`.

## Current Landscape

- **Routes**: `apps/backend/src/routes/auth.ts` handles login flows, callbacks, logout, and current-user endpoint.
- **Services**:
  - `services/auth.service.ts` – issues sessions, persists them via storage.
  - `services/github.service.ts` and `services/auth0.service.ts` – provider-specific helpers.
  - `services/access-control/context-session-provider.ts` – AsyncLocalStorage wrapper.
- **Middleware**: `middleware/auth.ts` and `middleware/session-context.ts` secure API routes and expose session info.
- **Storage**: session data, OAuth tokens, and group mappings persist via the shared storage backend.
- **Shared usage**: repositories, settings, and other features call GitHub/Auth services for tokens and user info.

## Target Structure

```
apps/backend/src/features/auth/
  api/
    auth.routes.ts
  services/
    session.service.ts
    oauth.service.ts
  types/
    session.types.ts
    auth.types.ts
  utils/
    token.utils.ts
  index.ts

apps/backend/src/infrastructure/clients/
  github.client.ts
  auth0.client.ts

features/auth/storage/
  session.storage.ts
```

- Contracts & schemas live in `packages/api-contract/src/auth/auth.contract.ts` and `packages/api-contract/src/auth/auth.schemas.ts`.
- Shared middleware moves to `apps/backend/src/shared/http/middleware/` with exports wired through the auth feature factory.
- `features/auth/index.ts` exports factories (`createAuthFeature`) returning route registration + service facades (`AuthFacade`, `SessionFacade`).

## Migration Phases

### Phase 1 – Contracts & Routing

1. Create new contract + schema files under `packages/api-contract/src/auth/` mirroring existing endpoints.
2. Add unit tests verifying contracts compile and schemas validate happy-path payloads.
3. Introduce `features/auth/api/auth.routes.ts` consuming the new contracts and delegating to placeholder services (wrapping legacy code initially).
4. Update `app.ts` to register `features/auth/api/auth.routes.ts` instead of `routes/auth.ts`.

### Phase 2 – Services & Facades

1. Split `auth.service.ts` into:
   - `session.service.ts` – session issuance, cookie config, AsyncLocalStorage hooks.
   - `oauth.service.ts` – common OAuth flows (state generation, callback handling) delegating to provider clients.
2. Define interfaces for provider operations (`OAuthProviderClient`, `SessionStore`) in `types/`.
3. Retain existing business logic by moving code into the new services; expose stable facades (`AuthFacade`, `SessionFacade`) for cross-feature consumption.
4. Update dependents (repositories, settings, middleware) to consume the facades imported from `features/auth` rather than directly from legacy services.

### Phase 3 – Infrastructure & Storage Adapters

1. Move provider-specific logic into `apps/backend/src/infrastructure/clients/github.client.ts` and `auth0.client.ts`, implementing the interfaces defined in Phase 2.
2. Create `features/auth/storage/session.storage.ts` that composes the shared primitives from `apps/backend/src/infrastructure/storage/` to provide session-specific get/save/delete operations.
3. Wire infrastructure and storage implementations into the feature factory (`createAuthFeature`) so services receive dependencies through constructor parameters.
4. Remove old files under `services/github.service.ts`, `services/auth0.service.ts`, and `services/auth.service.ts` once references are updated.

### Phase 4 – Middleware & Session Context

1. Relocate `middleware/auth.ts` and `middleware/session-context.ts` into `shared/http/middleware/`.
2. Re-export configured middleware from `features/auth` so `app.ts` can apply them alongside route registration.
3. Ensure `SessionContext` AsyncLocalStorage helper lives within the auth feature (services or utils) and is re-exported for consumers still needing direct access.
4. Add integration tests verifying middleware + routes honour auth provider toggles (`AUTH_PROVIDER`, `SESSION_MAX_AGE`, `STORAGE_TYPE`).

### Phase 5 – Cleanup & Validation

1. Delete `apps/backend/src/routes/auth.ts` and old service files.
2. Update docs (`backend-feature-domains.md`, architecture pattern) to reflect completion.
3. Run full test suite; add feature-level README documenting new structure, exported APIs, environment variables, and intended usage.

## Dependencies & Considerations

- **Shared Storage**: confirm the primitives in `infrastructure/storage` cover session needs; avoid reintroducing a global session store.
- **Environment Variables**: centralise validation (e.g. in `shared/config/env.ts`).
- **Consumer Updates**: adjust repositories, settings, and workspace features to import the new `AuthFacade`.
- **Testing**: ensure end-to-end login flow (GitHub + Auth0) remains covered; consider adding contract tests verifying API responses match schemas.
- **Rollout Strategy**: migrate behind feature flag if necessary, or ensure atomic PR that keeps legacy code until new feature is stable.

## Deliverables

- `features/auth` directory structure with implemented services and utils.
- Updated `infrastructure/clients` entries and `features/auth/storage` adapters.
- Updated `packages/api-contract` auth contracts & schemas.
- Shared middleware relocated and wired through the feature index.
- Documentation & tests reflecting the new feature-based auth ownership.
