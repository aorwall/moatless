# OpenAPI + Shared Contract rollout

Goal: every HTTP endpoint is defined once in `packages/api-contract`, registered with `app.openapi()` in
the backend, and consumed through `buildPath`/`RouteSuccess` helpers in the frontend. This produces a
full `/openapi.json`, keeps clients typed, and removes string literals from the UI. For concrete code
Examples, refer to the **API Patterns (Quick Reference)** guide at
`docs/patterns/api-best-practices.md`.

---

## Foundation (done)
- Backend bootstrapped with `OpenAPIHono` + Scalar UI at `/docs`.
- `packages/api-contract` publishes route descriptors and helpers (`buildPath`, `RouteParams`,
  `RouteRequestJson`, `RouteSuccess`).
- Workspace feature already demonstrates the pattern end-to-end (descriptor → backend → frontend).

---

## Migration pattern (repeat per feature)
1. **Contract** – Create/extend descriptor(s) under `packages/api-contract/src/routes/<feature>.ts`.
   - Include method, path, params/query schemas, body schemas, `withStandardErrors`, and tags.
   - Export descriptor via `packages/api-contract/src/routes/index.ts`.
2. **Backend** – Import descriptor and register with `app.openapi(descriptor, handler)`.
   - Replace manual `createRoute` definitions with imports from the contract package.
   - Use `c.req.valid()` for params/query/body.
3. **Frontend** – Import descriptor + helpers, replace string paths with `buildPath(...)`, and infer
   request/response types via `RouteRequestJson` / `RouteSuccess` (see pattern doc for snippets).
4. **Tests & docs** – Ensure `bun run --filter=@moatless/backend type:check` and
   `bun run --filter=@moatless/frontend type:check` pass, check `/openapi.json`, and update storybook
   mocks if they rely on old shapes.

Keep migrations small (one feature folder at a time) so diffs remain reviewable.

---

## Feature rollout order

### ✅ Completed (57 routes - Backend only)
| Feature | Routes | Backend files | Status |
| --- | --- | --- | --- |
| Workspace | 13 routes | `apps/backend/routes/workspace.ts` | ✅ Complete |
| Settings | 9 routes | `routes/settings.ts`, `routes/settings/*.ts` | ✅ Complete |
| Preview | 5 routes | `routes/preview-environment.ts` | ✅ Complete |
| Commands | 5 routes | `routes/commands.ts` | ✅ Complete |
| Uploads | 2 routes | `routes/uploads.ts` | ✅ Complete |
| Usage | 2 routes | `routes/usage.ts` | ✅ Complete |
| Worker | 1 route | `routes/worker.ts` | ✅ Complete |
| Auth | 3 routes | `routes/auth.ts` | ✅ Complete |
| Repositories | 9 routes | `routes/repositories.ts` | ✅ Complete |
| Tasks | 15 routes | `routes/tasks.ts` | ✅ Complete |

**Note:** Backend migration complete. Frontend API clients still need migration.

### 🔄 Frontend Migration (57 routes)
All routes now have shared descriptors but frontend still uses string paths. Need to migrate:

| Priority | Feature | Frontend files | Complexity |
| --- | --- | --- | --- |
| 1 | Tasks | `features/tasks/api/tasks.ts` | High - 15 routes, complex queries |
| 2 | Repositories | `features/repositories/api/*.ts` | Medium - 9 routes |
| 3 | Workspace | Already migrated | ✅ Complete |
| 4 | Settings | `features/settings/api/*.ts` | Low - 9 routes |
| 5 | Auth | `features/auth/api/*.ts` | Low - 3 routes |
| 6 | Preview | `features/preview/api/*.ts` | Low - 5 routes |
| 7 | Commands | `features/commands/api/*.ts` | Low - 5 routes |
| 8 | Uploads | `features/uploads/api/*.ts` | Low - 2 routes |
| 9 | Usage | `features/usage/api/*.ts` | Low - 2 routes |
| 10 | Worker | `features/worker/api/*.ts` | Low - 1 route |

---

## Frontend Migration Strategy

### Phase 1: Backend complete ✅
All backend routes now use shared descriptors from `@moatless/api-contract`.

### Phase 2: Frontend API clients (current)
Replace string-based paths with typed `buildPath()` calls. Follow patterns in
`docs/patterns/api-best-practices.md`.

#### Migration pattern per feature:
1. **Import descriptors** from `@moatless/api-contract/routes/<feature>`
2. **Replace hardcoded paths** with `buildPath(descriptor, params)`
3. **Type request bodies** with `RouteRequestJson<typeof descriptor>`
4. **Type responses** with `RouteSuccess<typeof descriptor>`
5. **Update React Query keys** to use descriptor-based keys
6. **Test** that all API calls still work

#### Example transformation:

**Before:**
```typescript
export const fetchTask = async (taskId: string): Promise<Task> => {
  const response = await apiClient.get(`/api/v1/tasks/${taskId}`)
  return response.data
}
```

**After:**
```typescript
import { getTaskRoute } from '@moatless/api-contract/routes/tasks'
import { buildPath, type RouteSuccess } from '@moatless/api-contract'

export const fetchTask = async (
  taskId: string
): Promise<RouteSuccess<typeof getTaskRoute>> => {
  const url = buildPath(getTaskRoute, { taskId })
  const response = await apiClient.get(url)
  return response.data
}
```

#### Key benefits:
- **Type safety**: Params, body, and response types all inferred from descriptors
- **Refactor-safe**: Changing a path in one place updates everywhere
- **IDE support**: Autocomplete for params and response fields
- **Runtime validation**: Impossible to pass wrong param types

---

## Per-feature checklist (Backend)
- [x] Add or update descriptors in `packages/api-contract/src/routes/<feature>.ts`
- [x] Export descriptors via `packages/api-contract/src/routes/index.ts`
- [x] Replace inline `createRoute` calls in backend with descriptor imports
- [x] Use `app.openapi(descriptor, handler)` pattern
- [x] Build api-contract package
- [x] Verify type-check passes

## Per-feature checklist (Frontend - NEW)
- [ ] Import route descriptors from `@moatless/api-contract/routes/<feature>`
- [ ] Replace string paths with `buildPath(descriptor, params)`
- [ ] Type request bodies with `RouteRequestJson<typeof descriptor>`
- [ ] Type responses with `RouteSuccess<typeof descriptor>`
- [ ] Update React Query hooks to use typed responses
- [ ] Update query keys to be descriptor-based
- [ ] Test all API calls in the feature
- [ ] Commit before moving to next feature

---

## Completion criteria
- All routes live in `packages/api-contract` descriptors.
- Backend uses descriptors exclusively with `app.openapi()`; no ad-hoc paths remain.
- Frontend API modules rely solely on shared descriptors for URLs and types.
- `/openapi.json` and Scalar UI include every endpoint with accurate documentation.
- Type-check scripts for backend, frontend, and api-contract succeed without manual overrides.

Track progress feature-by-feature; once every row in the rollout table is checked off, the migration is
done.
