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

### ✅ Completed (27 routes)
| Feature | Routes | Backend files | Status |
| --- | --- | --- | --- |
| Workspace | 13 routes | `apps/backend/routes/workspace.ts` | ✅ Complete |
| Settings | 9 routes | `routes/settings.ts`, `routes/settings/*.ts` | ✅ Complete |
| Preview | 5 routes | `routes/preview-environment.ts` | ✅ Complete |
| Commands | 5 routes | `routes/commands.ts` | ✅ Complete |
| Uploads | 2 routes | `routes/uploads.ts` | ✅ Complete |
| Usage | 2 routes | `routes/usage.ts` | ✅ Complete |
| Worker | 1 route | `routes/worker.ts` | ✅ Complete |

### 🔄 Remaining (33 routes)
| Order | Feature | Routes | Backend files | Frontend consumers |
| --- | --- | --- | --- | --- |
| 1 | Auth | 3 routes | `routes/auth.ts` | login/onboarding flows |
| 2 | Repositories | ~12 routes | `routes/repositories.ts` | `features/repositories/api/*.ts` |
| 3 | Tasks | ~18 routes | `routes/tasks.ts` | `features/tasks/api/tasks.ts` |

**Note:** Auth uses dev mode logic that must be preserved during migration.

---

## Per-feature checklist
- [ ] Add or update descriptors in `packages/api-contract/src/routes/<feature>.ts` (include tags and
      descriptions for OpenAPI clarity).
- [ ] Export descriptors via `packages/api-contract/src/routes/index.ts` and re-run
      `bun run --filter=@moatless/api-contract build`.
- [ ] Replace inline `createRoute` calls in backend with descriptor imports and
      `app.openapi(descriptor, handler)`.
- [ ] Update frontend API layer(s) to call `buildPath(descriptor, params)` and infer types from
      `RouteSuccess` / `RouteRequestJson`.
- [ ] Adjust tests, stories, and fixtures that referenced the old shapes or URLs.
- [ ] Verify `/openapi.json` lists the migrated endpoints with correct schemas and tags.
- [ ] Commit before moving to the next feature.

---

## Completion criteria
- All routes live in `packages/api-contract` descriptors.
- Backend uses descriptors exclusively with `app.openapi()`; no ad-hoc paths remain.
- Frontend API modules rely solely on shared descriptors for URLs and types.
- `/openapi.json` and Scalar UI include every endpoint with accurate documentation.
- Type-check scripts for backend, frontend, and api-contract succeed without manual overrides.

Track progress feature-by-feature; once every row in the rollout table is checked off, the migration is
done.
