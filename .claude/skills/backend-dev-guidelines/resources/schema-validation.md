# Schema Validation with Zod

## Overview
All API input is validated using Zod schemas defined in `@moatless/types`.

## Basic Schemas

```tsx
// packages/types/src/schemas/task.schema.ts
import { z } from 'zod'

export const TaskSchema = z.object({
  id: z.string().uuid(),
  title: z.string().min(1).max(255),
  description: z.string().optional(),
  status: z.enum(['pending', 'running', 'completed', 'failed']),
  createdAt: z.date(),
  updatedAt: z.date()
})

export const CreateTaskSchema = TaskSchema.omit({
  id: true,
  createdAt: true,
  updatedAt: true
})

export const UpdateTaskSchema = CreateTaskSchema.partial()

// Export types
export type Task = z.infer<typeof TaskSchema>
export type CreateTask = z.infer<typeof CreateTaskSchema>
export type UpdateTask = z.infer<typeof UpdateTaskSchema>
```

## Validation in Routes

### Automatic Validation
```tsx
import { zValidator } from '@hono/zod-validator'
import { CreateTaskSchema } from '@moatless/types'

// OpenAPI routes validate automatically
app.openapi(routes.tasks.create, async (c) => {
  const body = c.req.valid('json')  // Already validated!
  return c.json(body, 201)
})

// Manual validation for non-OpenAPI routes
app.post('/tasks', zValidator('json', CreateTaskSchema), async (c) => {
  const body = c.req.valid('json')  // Validated!
  return c.json(body, 201)
})
```

## Complex Validation

### Nested Objects
```tsx
const TaskWithSubtasksSchema = z.object({
  title: z.string().min(1),
  description: z.string().optional(),
  subtasks: z.array(z.object({
    title: z.string().min(1),
    order: z.number().int().positive()
  }))
})
```

### Conditional Fields
```tsx
const TaskSchema = z.object({
  title: z.string(),
  type: z.enum(['simple', 'recurring']),
  schedule: z.string().optional()
}).refine(
  (data) => data.type !== 'recurring' || data.schedule !== undefined,
  { message: 'Schedule required for recurring tasks' }
)
```

### Custom Validation
```tsx
const CreateTaskSchema = z.object({
  title: z.string().min(3, 'Title must be at least 3 characters'),
  dueDate: z.string().refine((val) => {
    const date = new Date(val)
    return date > new Date()
  }, 'Due date must be in the future')
})
```

## Response Validation

```tsx
import { createRoute, z } from '@hono/zod-openapi'

export const routes = {
  tasks: {
    create: createRoute({
      method: 'post',
      path: '/api/v1/tasks',
      request: {
        body: {
          content: {
            'application/json': {
              schema: CreateTaskSchema  // Request validation
            }
          }
        }
      },
      responses: {
        201: {
          content: {
            'application/json': {
              schema: TaskSchema  // Response validation
            }
          }
        }
      }
    })
  }
}
```

## Error Messages

### Custom Error Messages
```tsx
const TaskSchema = z.object({
  title: z.string()
    .min(1, 'Title is required')
    .max(255, 'Title must be less than 255 characters'),
  priority: z.number()
    .int('Priority must be an integer')
    .min(1, 'Priority must be at least 1')
    .max(5, 'Priority must be at most 5')
})
```

### Handling Validation Errors
```tsx
app.post('/tasks', async (c) => {
  try {
    const body = await c.req.json()
    const validated = CreateTaskSchema.parse(body)
    // Use validated data
  } catch (error) {
    if (error instanceof z.ZodError) {
      return c.json({
        error: 'Validation failed',
        details: error.errors
      }, 400)
    }
    throw error
  }
})
```
