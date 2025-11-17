# Error Handling

## Overview
Consistent error handling across routes and services using typed errors.

## Error Types

```tsx
// apps/backend/src/utils/errors.ts
export class AppError extends Error {
  constructor(
    message: string,
    public statusCode: number = 500,
    public code?: string
  ) {
    super(message)
    this.name = 'AppError'
  }
}

export class NotFoundError extends AppError {
  constructor(message: string) {
    super(message, 404, 'NOT_FOUND')
  }
}

export class ValidationError extends AppError {
  constructor(message: string) {
    super(message, 400, 'VALIDATION_ERROR')
  }
}

export class UnauthorizedError extends AppError {
  constructor(message: string) {
    super(message, 401, 'UNAUTHORIZED')
  }
}
```

## Service Layer Errors

```tsx
@injectable()
export class TaskService {
  async getTaskById(id: string): Promise<Task> {
    const task = await this.taskRepo.findById(id)
    if (!task) {
      throw new NotFoundError(`Task with id ${id} not found`)
    }
    return task
  }

  async createTask(data: CreateTaskData): Promise<Task> {
    if (data.title.length < 3) {
      throw new ValidationError('Title must be at least 3 characters')
    }
    return this.taskRepo.create(data)
  }
}
```

## Route Error Handling

```tsx
// apps/backend/src/utils/error-handler.ts
import type { Context } from 'hono'
import { AppError } from './errors'

export function handleServiceError(c: Context, error: unknown) {
  if (error instanceof AppError) {
    return c.json({
      error: error.message,
      code: error.code
    }, error.statusCode)
  }

  console.error('Unexpected error:', error)
  return c.json({
    error: 'Internal server error'
  }, 500)
}

// Usage in routes
app.openapi(routes.tasks.detail, async (c) => {
  try {
    const { taskId } = c.req.valid('param')
    const taskService = appContainer.get<TaskService>(TYPES.TaskService)
    const task = await taskService.getTaskById(taskId)
    return c.json(task, 200)
  } catch (error) {
    return handleServiceError(c, error)
  }
})
```

## Global Error Handler

```tsx
// apps/backend/src/app.ts
app.onError((err, c) => {
  if (err instanceof AppError) {
    return c.json({
      error: err.message,
      code: err.code
    }, err.statusCode)
  }

  console.error('Unhandled error:', err)
  return c.json({
    error: 'Internal server error'
  }, 500)
})
```
