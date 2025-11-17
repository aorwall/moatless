# API Contract Pattern

## Overview
This project uses **OpenAPI-first contract design** where all API routes are defined in the `@moatless/api-contract` package before implementation.

## Key Principles
- Define contracts before implementation
- Single source of truth for API structure
- Type-safe path building
- Automatic OpenAPI documentation

## Contract Definition

### Basic Route Contract
```tsx
// packages/api-contract/src/routes/tasks.ts
import { createRoute, z } from '@hono/zod-openapi'

export const TaskSchema = z.object({
  id: z.string(),
  title: z.string(),
  status: z.enum(['pending', 'running', 'completed'])
})

export const routes = {
  tasks: {
    list: createRoute({
      method: 'get',
      path: '/api/v1/tasks',
      responses: {
        200: {
          content: {
            'application/json': {
              schema: z.array(TaskSchema)
            }
          },
          description: 'List of tasks'
        }
      }
    }),

    create: createRoute({
      method: 'post',
      path: '/api/v1/tasks',
      request: {
        body: {
          content: {
            'application/json': {
              schema: z.object({
                title: z.string().min(1),
                description: z.string().optional()
              })
            }
          }
        }
      },
      responses: {
        201: {
          content: {
            'application/json': {
              schema: TaskSchema
            }
          },
          description: 'Created task'
        }
      }
    }),

    detail: createRoute({
      method: 'get',
      path: '/api/v1/tasks/{taskId}',
      request: {
        params: z.object({
          taskId: z.string()
        })
      },
      responses: {
        200: {
          content: {
            'application/json': {
              schema: TaskSchema
            }
          },
          description: 'Task details'
        },
        404: {
          description: 'Task not found'
        }
      }
    })
  }
}
```

## Type Helpers

### RouteSuccess Type
Extract response type from route:
```tsx
import { RouteSuccess } from '@moatless/api-contract'

type TaskListResponse = RouteSuccess<typeof routes.tasks.list>
// Result: Task[]

type TaskDetailResponse = RouteSuccess<typeof routes.tasks.detail>
// Result: Task
```

### RouteParams Type
Extract path parameters:
```tsx
import { RouteParams } from '@moatless/api-contract'

type TaskParams = RouteParams<typeof routes.tasks.detail>
// Result: { taskId: string }
```

### RouteRequestJson Type
Extract request body type:
```tsx
import { RouteRequestJson } from '@moatless/api-contract'

type CreateTaskRequest = RouteRequestJson<typeof routes.tasks.create>
// Result: { title: string; description?: string }
```

## Path Building

### buildPath Function
ALWAYS use `buildPath` to construct URLs:

```tsx
import { buildPath, routes } from '@moatless/api-contract'

// Simple path
const listUrl = buildPath(routes.tasks.list)
// Result: '/api/v1/tasks'

// Path with params
const detailUrl = buildPath(routes.tasks.detail, { taskId: '123' })
// Result: '/api/v1/tasks/123'

// Path with query params
const filteredUrl = buildPath(routes.tasks.list, {}, { status: 'pending' })
// Result: '/api/v1/tasks?status=pending'
```

## Implementation Pattern

### Backend Route Handler
```tsx
// apps/backend/src/routes/tasks.ts
import { OpenAPIHono } from '@hono/zod-openapi'
import { routes } from '@moatless/api-contract'
import { appContainer } from '../container'
import { TYPES } from '../types'
import type { TaskService } from '../services/task.service'

const app = new OpenAPIHono()

// List tasks
app.openapi(routes.tasks.list, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const tasks = await taskService.getAllTasks()
  return c.json(tasks, 200)
})

// Create task
app.openapi(routes.tasks.create, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const body = c.req.valid('json')
  const task = await taskService.createTask(body)
  return c.json(task, 201)
})

// Get task detail
app.openapi(routes.tasks.detail, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const { taskId } = c.req.valid('param')
  const task = await taskService.getTaskById(taskId)

  if (!task) {
    return c.json({ error: 'Task not found' }, 404)
  }

  return c.json(task, 200)
})

export default app
```

### Frontend API Client
```tsx
// apps/frontend/src/features/tasks/api/use-tasks-query.ts
import { useQuery } from '@tanstack/react-query'
import { buildPath, routes, RouteSuccess } from '@moatless/api-contract'
import { apiClient } from '@/lib/api/client'

type TaskListResponse = RouteSuccess<typeof routes.tasks.list>

export function useTasksQuery() {
  return useQuery({
    queryKey: ['tasks'],
    queryFn: async () => {
      const url = buildPath(routes.tasks.list)
      const response = await apiClient.get<TaskListResponse>(url)
      return response.data
    }
  })
}
```

## Error Responses

Define error schemas:
```tsx
const ErrorSchema = z.object({
  error: z.string(),
  details: z.record(z.any()).optional()
})

export const routes = {
  tasks: {
    create: createRoute({
      method: 'post',
      path: '/api/v1/tasks',
      request: { /* ... */ },
      responses: {
        201: { /* success */ },
        400: {
          content: {
            'application/json': {
              schema: ErrorSchema
            }
          },
          description: 'Validation error'
        },
        500: {
          content: {
            'application/json': {
              schema: ErrorSchema
            }
          },
          description: 'Server error'
        }
      }
    })
  }
}
```

## Anti-Patterns

❌ **Hardcoding paths**
```tsx
// WRONG
fetch('/api/v1/tasks')

// CORRECT
import { buildPath, routes } from '@moatless/api-contract'
fetch(buildPath(routes.tasks.list))
```

❌ **Manual type definitions**
```tsx
// WRONG
interface Task {
  id: string
  title: string
}

// CORRECT - Use contract types
import { RouteSuccess, routes } from '@moatless/api-contract'
type Task = RouteSuccess<typeof routes.tasks.detail>
```

❌ **Skipping contract definition**
```tsx
// WRONG - Route not in contract
app.get('/api/v1/secret-tasks', async (c) => { ... })

// CORRECT - Define in contract first
// 1. Add to @moatless/api-contract
// 2. Then implement route
```
