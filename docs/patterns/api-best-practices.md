# API Patterns (Quick Reference)

The backend is schema-first, OpenAPI-backed, and route descriptors are shared with clients. Follow
these steps whenever you add or update endpoints.

---

## 1. Sources of Truth

| Domain | Location | Notes |
| --- | --- | --- |
| **Validation + Types** | `packages/types/src/schemas/*.ts` | Zod schemas; types come from `z.infer`. |
| **Shared Contract** | `packages/api-contract/src/routes/*.ts` | `createRoute()` descriptors consumed by backend + frontend. |
| **Helpers** | `packages/api-contract/src/helpers.ts` | `buildPath`, `RouteParams`, `RouteSuccess`, `RouteRequestJson`. |
| **Backend Utilities** | `apps/backend/utils/*` | `handleServiceError`, `validateRequestBody`, `pathParams`, `withStandardErrors`. |
| **HTTP Client** | `apps/frontend/src/lib/api/client.ts` | Standard fetch wrapper with JSON handling. |

Keep schemas pure (no environment access) and import them everywhere else.

---

## 2. Define / Update a Route Descriptor

```ts
// packages/api-contract/src/routes/workspace.ts
import { createRoute } from '@hono/zod-openapi'
import { WorkspaceStatusResponseSchema } from '@moatless/types'
import { pathParams } from '../params.js'
import { withStandardErrors } from '../responses.js'

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  request: { params: pathParams.taskId },
  responses: withStandardErrors(
    WorkspaceStatusResponseSchema,
    'Workspace status with environment, git and capability details',
  ),
})
```

Rules:
- Export every contract from `packages/api-contract/src/routes/index.ts`.
- Always use `pathParams` shortcuts and `withStandardErrors` for consistency.
- Run `bun run --filter=@moatless/api-contract build` to ensure the descriptor type-checks.

---

## 3. Backend Wiring

```ts
// apps/backend/routes/workspace.ts
import { OpenAPIHono } from '@hono/zod-openapi'
import { getWorkspaceStatusRoute } from '@moatless/api-contract/routes/workspace'
import { handleServiceError } from '../utils/error-handler.js'

export async function registerWorkspaceRoutes(app: OpenAPIHono<AppBindings>) {
  app.openapi(getWorkspaceStatusRoute, async (c) => {
    try {
      const services = c.get('services')
      const { taskId } = c.req.valid('param')
      return c.json(await services.workspace.getWorkspaceStatus(taskId))
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })
}
```

Checklist:
1. Use `OpenAPIHono` and `app.openapi(route, handler)` so descriptors feed `/openapi.json`.
2. Pull validated input with `c.req.valid('param' | 'json' | 'query')`.
3. Wrap logic in `try/catch` and call `handleServiceError`.
4. Let shared helpers (`validateRequestBody`, `validateQueryParams`) do parsing when needed.

---

## 4. Frontend Consumption

```ts
// apps/frontend/src/features/tasks/api/tasks.ts
import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import { getWorkspaceStatusRoute } from '@moatless/api-contract/routes/workspace'
import { apiClient } from '@/lib/api/client'

type WorkspaceStatusResponse = RouteSuccess<typeof getWorkspaceStatusRoute>
	export async function getTaskWorkspaceStatus(
  taskId: RouteParams<typeof getWorkspaceStatusRoute>['taskId'],
): Promise<WorkspaceStatusResponse> {
  return apiClient.get(buildPath(getWorkspaceStatusRoute, { taskId }))
}
```

Guidelines:
- Never hard-code `/api/v1/...`; always call `buildPath(route, params)`.
- Use `RouteSuccess` / `RouteRequestJson` / `RouteParams` to infer types.
- API client automatically throws `ApiError`; callers should handle it or let React Query error flow.

---

## 5. Utility Highlights

- **`handleServiceError`** (`apps/backend/utils/error-handler.ts`): translates domain/service errors to
  HTTP responses (400/404/500). Always use it in backend routes.
- **`withStandardErrors`** (`packages/api-contract/src/responses.ts`): attaches `400/404/500` docs.
- **`validateRequestBody` / `validateQueryParams`** (`apps/backend/utils/route-helpers.ts`): run Zod
  validation with consistent error messaging.
- **`pathParams`** (`packages/api-contract/src/params.ts`): pre-built Zod schemas for ids.
- **Testing** (`apps/backend/test/helper.ts`): use `OpenAPIHono` when instantiating route apps in tests
  so type signatures match production.

---

## 6. New Endpoint Checklist

1. **Schema**: add/adjust Zod schema + exports in `packages/types`.
2. **Contract**: create/update descriptor in `packages/api-contract` (method, path, request, responses).
3. **Backend**: import descriptor, wire handler with `app.openapi`, call services, and use
   `handleServiceError`.
4. **Frontend**: import descriptor + helpers, use `buildPath`, infer request/response types.
5. **Docs/Tags**: add meaningful `tags` + `description` to the descriptor for OpenAPI clarity.
6. **Type-check**: `bun run --filter=@moatless/api-contract build`, `bun run --filter=@moatless/backend type:check`, `bun run --filter=@moatless/frontend type:check`.

Follow these steps and both the OpenAPI spec and the clients stay accurate without duplicate
literals or manual type definitions.
