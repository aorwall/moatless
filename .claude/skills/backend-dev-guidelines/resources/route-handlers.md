# Route Handlers

## Overview
Route handlers in Hono with OpenAPI integration handle HTTP concerns only - no business logic.

## Basic Route Pattern

```tsx
import { OpenAPIHono } from '@hono/zod-openapi'
import { routes } from '@moatless/api-contract'
import { appContainer } from '../container'
import { TYPES } from '../types'

const app = new OpenAPIHono()

app.openapi(routes.tasks.list, async (c) => {
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const tasks = await taskService.getAllTasks()
  return c.json(tasks, 200)
})

export default app
```

## Request Validation

### Path Parameters
```tsx
app.openapi(routes.tasks.detail, async (c) => {
  const { taskId } = c.req.valid('param')  // Validated!
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const task = await taskService.getTaskById(taskId)
  return c.json(task, 200)
})
```

### JSON Body
```tsx
app.openapi(routes.tasks.create, async (c) => {
  const body = c.req.valid('json')  // Validated by Zod schema!
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const task = await taskService.createTask(body)
  return c.json(task, 201)
})
```

### Query Parameters
```tsx
app.openapi(routes.tasks.list, async (c) => {
  const { status, page } = c.req.valid('query')
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)
  const tasks = await taskService.getTasks({ status, page })
  return c.json(tasks, 200)
})
```

## Error Handling

### Service Errors
```tsx
import { handleServiceError } from '../utils/error-handler'

app.openapi(routes.tasks.detail, async (c) => {
  try {
    const { taskId } = c.req.valid('param')
    const taskService = appContainer.get<TaskService>(TYPES.TaskService)
    const task = await taskService.getTaskById(taskId)

    if (!task) {
      return c.json({ error: 'Task not found' }, 404)
    }

    return c.json(task, 200)
  } catch (error) {
    return handleServiceError(c, error)
  }
})
```

### Custom Error Responses
```tsx
app.openapi(routes.tasks.delete, async (c) => {
  const { taskId } = c.req.valid('param')
  const taskService = appContainer.get<TaskService>(TYPES.TaskService)

  try {
    await taskService.deleteTask(taskId)
    return c.json({ message: 'Task deleted' }, 200)
  } catch (error) {
    if (error.message === 'Task not found') {
      return c.json({ error: 'Task not found' }, 404)
    }
    if (error.message === 'Cannot delete running task') {
      return c.json({ error: error.message }, 400)
    }
    throw error
  }
})
```

## Route Registration

```tsx
// apps/backend/src/app.ts
import { OpenAPIHono } from '@hono/zod-openapi'
import tasksRoutes from './routes/tasks'
import environmentsRoutes from './routes/environments'

const app = new OpenAPIHono()

// Register routes
app.route('/api/v1/tasks', tasksRoutes)
app.route('/api/v1/environments', environmentsRoutes)

// OpenAPI documentation
app.doc('/api/v1/openapi.json', {
  openapi: '3.0.0',
  info: {
    title: 'Moatless Swarm API',
    version: '1.0.0'
  }
})

export default app
```

## Middleware

### Global Middleware
```tsx
import { logger } from './plugins/logger'

app.use('*', logger())
app.use('*', cors())
```

### Route-Specific Middleware
```tsx
import { authMiddleware } from './middleware/auth'

app.use('/api/v1/tasks/*', authMiddleware)

app.openapi(routes.tasks.list, async (c) => {
  const user = c.get('user')  // Set by middleware
  // ...
})
```

## Anti-Patterns

❌ **Business logic in routes**
```tsx
// WRONG
app.post('/tasks', async (c) => {
  const body = await c.req.json()

  // Business logic belongs in service!
  if (body.title.length < 3) {
    return c.json({ error: 'Title too short' }, 400)
  }

  const task = await taskRepo.create(body)
  return c.json(task, 201)
})
```

❌ **Direct repository access**
```tsx
// WRONG - Skipping service layer
app.get('/tasks', async (c) => {
  const taskRepo = appContainer.get<TaskRepository>(TYPES.TaskRepository)
  const tasks = await taskRepo.findAll()  // Should call service!
  return c.json(tasks)
})
```

## Related Patterns
- [API Contract Pattern](./api-contract-pattern.md) - Route definitions
- [Layered Architecture](./layered-architecture.md) - Layer responsibilities
- [Error Handling](./error-handling.md) - Error responses
