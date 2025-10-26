# API Patterns (Quick Reference)

The backend is schema-first, OpenAPI-backed, and route descriptors are shared with clients. Follow
these steps whenever you add or update endpoints.

## Prerequisites

Before reading this pattern:
1. Review [Schema Design Patterns](schema-design-patterns.md) for Zod validation and type inference
2. Understand [Layered Architecture](layered-architecture.md) for proper separation of routes/services
3. See [Error Handling Pattern](error-handling.md) for consistent error responses
4. Review [React Query Patterns](react-query.md) for frontend data fetching integration

---

## 1. Sources of Truth

| Domain | Location | Notes |
| --- | --- | --- |
| **Validation + Types** | `packages/types/src/schemas/*.ts` | Zod schemas; types come from `z.infer`. |
| **Shared Contract** | `packages/api-contract/src/routes/<feature>.ts` (re-exported as `@moatless/api-contract/<feature>`) | `createRoute()` descriptors consumed by backend + frontend. |
| **Helpers** | `packages/api-contract/src/helpers.ts` | `buildPath`, `RouteParams`, `RouteSuccess`, `RouteRequestJson`. |
| **Backend Utilities** | `apps/backend/utils/*` | `handleServiceError`, `validateRequestBody`, logging. |
| **HTTP Client** | `apps/frontend/src/lib/api/client.ts` | Standard fetch wrapper with JSON handling. |

Keep schemas pure (no environment access) and import them everywhere else.

---

## 2. Define / Update a Route Descriptor

```ts
// packages/api-contract/src/routes/workspace.ts
import { createRoute } from '@hono/zod-openapi'
import { WorkspaceStatusResponseSchema } from '@moatless/types'
import { pathParams } from '../params'

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  tags: ['Workspace'],
  description: 'Get workspace status with environment, git and capability details',
  request: { params: pathParams.taskId },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: WorkspaceStatusResponseSchema,
        },
      },
    },
  },
})
```

Rules:
- MUST export every contract from `packages/api-contract/src/routes/index.ts`
- MUST use centralized param schemas from `packages/api-contract/src/params.ts` (e.g., `pathParams.taskId`, `pathParams.repositoryId`)
- MUST include `tags` and `description` fields for OpenAPI documentation
- MUST define responses inline with explicit `200` status code, `description`, and `schema` wrapped in `content.application/json`
- MUST use `204: { description: '...' }` without content for No Content responses
- SHOULD provide structured error responses (e.g., `{ error: ApiErrorSchema }`) for each failure scenario clients need to handle
- MAY include other success codes when appropriate (`201` for creates, pagination metadata wrappers, etc.)
- MUST run `bun run --filter=@moatless/api-contract build` to ensure the descriptor type-checks

### Example: POST Route with JSON Body

```ts
// packages/api-contract/src/routes/tasks.ts
import { createRoute } from '@hono/zod-openapi'
import { ApiErrorSchema, CreateTaskRequestSchema, TaskSchema } from '@moatless/types'
import { pathParams } from '../params'

export const createWorkspaceTaskRoute = createRoute({
  method: 'post',
  path: '/api/v1/workspaces/{workspaceId}/tasks',
  tags: ['Tasks'],
  description: 'Create a task inside a workspace',
  request: {
    params: pathParams.workspaceId,
    body: {
      content: {
        'application/json': {
          schema: CreateTaskRequestSchema,
        },
      },
    },
  },
  responses: {
    201: {
      description: 'Task created',
      content: {
        'application/json': {
          schema: TaskSchema,
        },
      },
    },
    400: {
      description: 'Invalid payload',
      content: {
        'application/json': {
          schema: ApiErrorSchema,
        },
      },
    },
  },
})
```

## TypeScript Module Rules

- MUST use named exports for every contract, helper, and handler. NEVER use `export default` or namespace wrappers.
- MUST import local modules with relative paths (`./foo`, `../bar`) to keep the project graph explicit.
- MUST keep exports immutable—avoid `export let` and export getter functions when you need dynamic data.
- MUST declare decorators immediately above the symbol they decorate, with any JSDoc above the decorator.
- MUST NOT use `import type`/`export type` shims; regular imports satisfy both value and type needs.

```typescript
// ✅ CORRECT – named exports with relative imports
import { buildPath } from '../helpers'
import { TaskSchema } from '@moatless/types'

export const listTasksRoute = createRoute({
  method: 'get',
  path: buildPath.tasks(),
  // ...
})

// ❌ WRONG – default export and absolute path for workspace file
import foo from 'routes/workspace'
export default function makeRoute() {
  // ...
}
```

---

## 3. Backend Wiring

```ts
// apps/backend/routes/workspace.routes.ts
import { OpenAPIHono } from '@hono/zod-openapi'
import { getWorkspaceStatusRoute } from '@moatless/api-contract/workspace'
import { handleServiceError } from '../utils/error-handler'

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
1. MUST use `OpenAPIHono` and `app.openapi(route, handler)` so descriptors feed `/openapi.json`
2. MUST pull validated input with `c.req.valid('param' | 'json' | 'query')`
3. MUST wrap logic in `try/catch` and call `handleServiceError`
4. SHOULD use shared helpers (`validateRequestBody`, `validateQueryParams`) for complex parsing

---

## 4. Frontend Consumption

```ts
// apps/frontend/src/features/tasks/api/tasks.client.ts
import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  createWorkspaceTaskRoute,
  getWorkspaceStatusRoute,
} from '@moatless/api-contract/workspace'
import { apiClient } from '@/lib/api/client'

type WorkspaceStatusResponse = RouteSuccess<typeof getWorkspaceStatusRoute>

export async function getTaskWorkspaceStatus(
  taskId: RouteParams<typeof getWorkspaceStatusRoute>['taskId'],
): Promise<WorkspaceStatusResponse> {
  return apiClient.get<WorkspaceStatusResponse>(buildPath(getWorkspaceStatusRoute, { taskId }))
}

type CreateTaskPayload = RouteRequestJson<typeof createWorkspaceTaskRoute>

export async function createTask(
  workspaceId: RouteParams<typeof createWorkspaceTaskRoute>['workspaceId'],
  payload: CreateTaskPayload,
) {
  return apiClient.post<RouteSuccess<typeof createWorkspaceTaskRoute>>(
    buildPath(createWorkspaceTaskRoute, { workspaceId }),
    { json: payload },
  )
}
```

Guidelines:
- MUST use `buildPath(route, params)` NEVER hard-code `/api/v1/...` paths
- MUST use `RouteSuccess` / `RouteRequestJson` / `RouteParams` to infer types
- MUST pass generic type parameter to `apiClient.get<T>()` / `post<T>()` / `delete<T>()` for proper type inference
- API client automatically throws `ApiError`; callers SHOULD handle it or let React Query error flow

---

## 5. Frontend Error Handling

Frontend code should surface structured errors in a consistent way. The canonical patterns (hooks, toast usage, validation formatting) live in `docs/patterns/react-query.md`. When in doubt, follow that document so backend contracts and UI handling stay aligned.

---

## 6. Utility Highlights

- **`handleServiceError`** (`apps/backend/utils/error-handler.ts`): translates domain/service errors to
  HTTP responses (400/404/500). Always use it in backend routes.
- **`validateRequestBody` / `validateQueryParams`** (`apps/backend/utils/route-helpers.ts`): run Zod
  validation with consistent error messaging.
- **`useApiError`** (`apps/frontend/src/lib/hooks/use-api-error.ts`): formats API errors for user display,
  handles validation errors, and provides type guards.
- **Testing** (`apps/backend/test/helper.ts`): use `OpenAPIHono` when instantiating route apps in tests
  so type signatures match production.

---

## 7. New Endpoint Checklist

1. **Schema**: add/adjust Zod schema + exports in `packages/types`.
2. **Contract**: create/update descriptor in `packages/api-contract` (method, path, request, responses).
3. **Backend**: import descriptor, wire handler with `app.openapi`, call services, and use
   `handleServiceError`.
4. **Frontend**: import descriptor + helpers, use `buildPath`, infer request/response types.
5. **Error Handling**: ensure mutations call `useApiError` patterns documented in `docs/patterns/react-query.md`.
6. **Docs/Tags**: add meaningful `tags` + `description` to the descriptor for OpenAPI clarity.
7. **Type-check**: `bun run --filter=@moatless/api-contract build`, `bun run --filter=@moatless/backend type:check`, `bun run --filter=@moatless/frontend type:check`.

Follow these steps and both the OpenAPI spec and the clients stay accurate without duplicate
literals or manual type definitions.
