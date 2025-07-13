# API Patterns and Best Practices

## Overview
This document outlines the established patterns and best practices for the Moatless API, reflecting the current schema-first architecture that has been implemented throughout the codebase.

## Current Architecture ✅

### 1. Schema-First Type System
The project follows a **schema-first** approach where all types derive from Zod schemas:

```
packages/types/
├── schemas/                 # Primary source of truth
│   ├── api.schemas.ts      # API request/response schemas
│   ├── task.schemas.ts     # Task-related schemas  
│   ├── workspace.schemas.ts # Workspace schemas
│   ├── repository.schemas.ts # Repository schemas
│   ├── command.schemas.ts   # Command schemas
│   ├── settings.schemas.ts  # Settings schemas
│   └── index.ts            # Schema exports
├── types/                  # Re-exports from schemas
│   ├── api.ts             # Re-exports API types
│   ├── task.ts            # Re-exports task types
│   ├── workspace.ts       # Re-exports workspace types
│   └── index.ts           # All type exports
└── index.ts               # Main package export
```

### 2. Three-Layer Architecture
1. **Schemas** - Zod schemas for runtime validation and type generation
2. **Types** - TypeScript types derived from schemas using `z.infer`
3. **Routes** - API endpoints using schemas for validation and types for safety

### 3. Centralized Utilities ✅
- **Error Handling**: `apps/backend/utils/error-handler.ts`
- **Route Helpers**: `apps/backend/utils/route-helpers.ts`
- **Validation Helpers**: Built into route helpers

## Established Patterns

### 1. Schema Definition Pattern ✅

```typescript
// 1. Define schema first (in schemas/[domain].schemas.ts)
export const CreateTaskSchema = z.object({
  message: z.string().min(1),
  repositoryId: z.string(),
  agentType: z.string(),
  // ...
})

// 2. Derive type from schema
export type CreateTaskRequest = z.infer<typeof CreateTaskSchema>

// 3. Export both schema and type
export { CreateTaskSchema }
export type { CreateTaskRequest }
```

### 2. Route Implementation Pattern ✅

```typescript
// Import from types package
import { CreateTaskSchema, handleServiceError, getTaskId } from '@moatless/types'
import { validateRequestBody } from '../utils/route-helpers.js'

app.post('/tasks', 
  describeRoute({
    description: 'Create a new task',
    requestBody: {
      content: {
        'application/json': {
          schema: resolver(CreateTaskSchema),
        },
      },
    },
    responses: {
      200: { /* ... */ },
      400: { /* ... */ },
    },
  }),
  async (c) => {
    try {
      // Use validation helper
      const validated = await validateRequestBody(c, CreateTaskSchema)
      
      const task = await services.task.create(validated)
      return c.json(task)
    } catch (error) {
      // Use centralized error handling
      handleServiceError(error, 'Create task')
    }
  }
)
```

### 3. Error Handling Pattern ✅

```typescript
// Centralized in utils/error-handler.ts
export function handleServiceError(error: unknown, context: string): never {
  logger.error(`${context} failed`, error as Error)

  if (error instanceof ValidationError) {
    throw new HTTPException(400, { message: error.message, cause: error })
  }
  
  if (error instanceof TaskNotFoundError) {
    throw new HTTPException(404, { message: error.message, cause: error })
  }
  
  // ... other error types
  
  throw new HTTPException(500, { 
    message: error instanceof Error ? error.message : `Failed to ${context.toLowerCase()}`,
    cause: error 
  })
}
```

### 4. Type Export Strategy ✅

```typescript
// packages/types/types/index.ts
// Primary exports from schemas (schema-first approach)
export * from '../schemas/index.js'

// Additional type-only exports when needed
export type { 
  TaskListResponse,    // Enhanced with Task/Event references
  EventListResponse,   // Enhanced with Task/Event references
} from './api.js'

// Specific exports to avoid conflicts
export { WorkspaceStatus } from './workspace.js' // Enum for backward compatibility
```

## Current Implementation Status

### ✅ Completed
- **Schema Consolidation**: All schemas moved to centralized files
- **Type Derivation**: All types derived from schemas using `z.infer`
- **Error Handling**: Centralized error handling utility implemented
- **Route Helpers**: Common patterns extracted to utilities
- **Validation**: Consistent Zod validation across routes
- **Export Strategy**: Clean export hierarchy with conflict resolution

### ✅ Best Practices Established

#### Schema Design
```typescript
// ✅ Use strict schemas with proper constraints
export const CommandNameSchema = z
  .string()
  .min(1)
  .max(100)
  .regex(/^[a-zA-Z0-9_-]+$/, 'Invalid format')

// ✅ Compose schemas for reusability  
export const UpdateTaskSchema = CreateTaskSchema.partial()

// ✅ Use enums for fixed values
export const TaskStatusSchema = z.enum(['open', 'running', 'error', 'merged', 'closed'])
```

#### Route Organization
```typescript
// ✅ Consistent parameter extraction
const taskId = getTaskId(c)  // Uses centralized helper

// ✅ Consistent validation
const validated = await validateRequestBody(c, CreateTaskSchema)
const queryParams = validateQueryParams(c, FileQuerySchema)

// ✅ Consistent error handling
try {
  // ... route logic
} catch (error) {
  handleServiceError(error, 'Operation context')
}
```

#### OpenAPI Integration
```typescript
// ✅ Proper schema integration
app.post('/endpoint',
  describeRoute({
    requestBody: {
      content: {
        'application/json': {
          schema: resolver(RequestSchema),  // Direct schema usage
        },
      },
    },
    responses: {
      200: {
        content: {
          'application/json': {
            schema: resolver(ResponseSchema),  // Direct schema usage
          },
        },
      },
    },
  }),
  handler
)
```

## Frontend Integration Patterns

### 1. API Layer Usage ✅
```typescript
// ✅ Import types from types package
import type { CreateTaskRequest, TaskResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const tasksApi = {
  async createTask(request: CreateTaskRequest): Promise<TaskResponse> {
    return await apiClient.post<TaskResponse>('/api/v1/tasks', request)
  }
}
```

### 2. Form Validation ✅
```typescript
// ✅ Use schemas for form validation
import { zodResolver } from '@hookform/resolvers/zod'
import { CreateTaskSchema } from '@moatless/types/schemas'

const form = useForm<CreateTaskRequest>({
  resolver: zodResolver(CreateTaskSchema),
  defaultValues: { /* ... */ }
})
```

### 3. React Query Integration ✅
```typescript
// ✅ Proper typing with mutations
export function useCreateTask() {
  return useMutation({
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onSuccess: async (data) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() })
    },
    onError: (error) => {
      const message = getErrorMessage(error)  // Centralized error handling
      toast({ title: 'Failed to create task', description: message })
    }
  })
}
```

## Usage Guidelines

### For New Features
1. **Start with Schema**: Define in `packages/shared/schemas/[domain].schemas.ts`
2. **Derive Types**: Use `z.infer<typeof Schema>` 
3. **Export Both**: Export schema for validation, type for consumption
4. **Use in Routes**: Import schema for validation, type for parameters
5. **Error Handling**: Use `handleServiceError` for consistent responses

### For Frontend
1. **Import Types**: Use types from `@moatless/types`
2. **Form Validation**: Use schemas with `zodResolver`
3. **API Calls**: Type all requests/responses properly
4. **Error Handling**: Use `getErrorMessage` helper

### Migration Pattern (for future changes)
1. **Add Schema**: Create new schema in appropriate file
2. **Export Type**: Add `z.infer` type export
3. **Update Routes**: Replace inline validation with schema
4. **Update Frontend**: Update imports to use shared types
5. **Test**: Verify no breaking changes

## Code Examples

### Complete Feature Implementation
```typescript
// 1. Schema (packages/types/schemas/resource.schemas.ts)
export const CreateResourceSchema = z.object({
  name: z.string().min(1).max(255),
  type: z.enum(['type1', 'type2']),
  metadata: z.record(z.string()).optional(),
})

export const ResourceResponseSchema = z.object({
  id: z.string(),
  name: z.string(),
  type: z.string(),
  metadata: z.record(z.string()),
  createdAt: z.date(),
  updatedAt: z.date(),
})

export type CreateResourceRequest = z.infer<typeof CreateResourceSchema>
export type ResourceResponse = z.infer<typeof ResourceResponseSchema>

// 2. Route (apps/backend/routes/resources.ts)
export async function registerResourceRoutes(app: Hono<AppBindings>, prefix: string) {
  app.post(prefix,
    describeRoute({
      description: 'Create a new resource',
      requestBody: {
        content: {
          'application/json': {
            schema: resolver(CreateResourceSchema),
          },
        },
      },
      responses: {
        201: {
          description: 'Resource created',
          content: {
            'application/json': {
              schema: resolver(ResourceResponseSchema),
            },
          },
        },
        400: CommonResponses[400],
      },
    }),
    async (c) => {
      try {
        const validated = await validateRequestBody(c, CreateResourceSchema)
        const resource = await services.resource.create(validated)
        return c.json(resource, 201)
      } catch (error) {
        handleServiceError(error, 'Create resource')
      }
    }
  )
}

// 3. Frontend API (apps/frontend/src/features/resources/api/resources.ts)
import type { CreateResourceRequest, ResourceResponse } from '@moatless/types'

export const resourcesApi = {
  async createResource(request: CreateResourceRequest): Promise<ResourceResponse> {
    return await apiClient.post<ResourceResponse>('/api/v1/resources', request)
  }
}

// 4. Frontend Hook (apps/frontend/src/features/resources/hooks/use-resources.ts)
export function useCreateResource() {
  return useMutation({
    mutationFn: resourcesApi.createResource,
    onSuccess: async (data) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.resources.lists() })
    },
    onError: (error) => {
      const message = getErrorMessage(error)
      toast({ title: 'Failed to create resource', description: message })
    }
  })
}

// 5. Frontend Form (apps/frontend/src/features/resources/components/resource-form.tsx)
import { zodResolver } from '@hookform/resolvers/zod'
import { CreateResourceSchema } from '@moatless/types/schemas'

export function ResourceForm() {
  const form = useForm<CreateResourceRequest>({
    resolver: zodResolver(CreateResourceSchema),
    defaultValues: { name: '', type: 'type1' }
  })
  
  const createResource = useCreateResource()
  
  const onSubmit = (data: CreateResourceRequest) => {
    createResource.mutate(data)
  }
  
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(onSubmit)}>
        {/* Form fields */}
      </form>
    </Form>
  )
}
```

## Standards and Conventions ✅

### File Organization
- **Schemas**: `packages/shared/schemas/[domain].schemas.ts`
- **Types**: `packages/shared/types/[domain].ts` (re-exports)
- **Routes**: `apps/backend/routes/[domain].ts`
- **Frontend API**: `apps/frontend/src/features/[feature]/api/[feature].ts`
- **Frontend Hooks**: `apps/frontend/src/features/[feature]/hooks/use-[feature].ts`

### Naming Conventions
- **Schemas**: `PascalCaseSchema` (e.g., `CreateTaskSchema`)
- **Types**: `PascalCase` (e.g., `CreateTaskRequest`)
- **Files**: `kebab-case` (e.g., `task.schemas.ts`)
- **Exports**: Both schema and type exported from same file

### Error Handling
- **Backend**: Use `handleServiceError(error, context)`
- **Frontend**: Use `getErrorMessage(error)` for user-friendly messages
- **Types**: Custom error classes extending `MoatlessError`

This architecture provides a robust foundation for API development with strong type safety, runtime validation, and excellent developer experience. The schema-first approach ensures consistency between backend validation and frontend types while maintaining a single source of truth for all API contracts.