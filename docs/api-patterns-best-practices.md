# API Patterns and Best Practices

## Overview
This document outlines the patterns, best practices, and improvement recommendations for the Moatless API based on analysis of the current types, schemas, and routes implementation.

## Current Architecture

### 1. Three-Layer Type System
The project uses a three-layer type system:

1. **Schemas** (`packages/shared/schemas/`) - Zod schemas for validation
2. **Types** (`packages/shared/types/`) - TypeScript interfaces and types
3. **Routes** (`apps/backend/routes/`) - API endpoint implementations

### 2. Key Patterns Observed

#### 2.1 Schema-First Approach
- Zod schemas define the structure and validation rules
- Types are derived from schemas using `z.infer`
- Routes use schemas for request/response validation

#### 2.2 Modular Route Organization
- Each domain has its own route file (tasks, repositories, etc.)
- Routes are registered with prefixes in `index.ts`
- Common patterns for CRUD operations

#### 2.3 Error Handling
- Custom error classes extending `MoatlessError`
- Consistent error codes and status codes
- Error handling helpers in routes

## Best Practices

### 1. Type and Schema Organization

#### DO:
```typescript
// 1. Define schema first
export const CreateTaskSchema = z.object({
  message: z.string(),
  repositoryId: z.string(),
  // ...
})

// 2. Derive type from schema
export type CreateTaskRequest = z.infer<typeof CreateTaskSchema>

// 3. Use in routes with validation
app.post('/tasks', async (c) => {
  const body = await c.req.json()
  const validated = CreateTaskSchema.parse(body)
  // ...
})
```

#### DON'T:
- Define types separately from schemas (causes duplication)
- Skip validation in routes
- Use `any` types in schemas

### 2. Route Structure

#### DO:
```typescript
// Consistent error handling
function handleServiceError(error: unknown, context: string): never {
  if (error instanceof ValidationError) {
    throw new HTTPException(400, { message: error.message })
  }
  if (error instanceof TaskNotFoundError) {
    throw new HTTPException(404, { message: error.message })
  }
  // ...
}

// Use OpenAPI descriptions
app.get(
  '/resource/:id',
  describeRoute({
    description: 'Get resource by ID',
    parameters: [/* ... */],
    responses: {
      200: { /* ... */ },
      404: { /* ... */ }
    }
  }),
  async (c) => { /* ... */ }
)
```

#### DON'T:
- Duplicate error handling logic
- Skip OpenAPI documentation
- Mix business logic with route handlers

### 3. Schema Design

#### DO:
```typescript
// Use strict schemas with proper constraints
export const CommandNameSchema = z
  .string()
  .min(1)
  .max(100)
  .regex(/^[a-zA-Z0-9_-]+$/, 'Invalid format')

// Compose schemas for reusability
export const UpdateSchema = BaseSchema.partial()

// Use enums for fixed values
export const StatusSchema = z.enum(['open', 'running', 'error', 'merged', 'closed'])
```

#### DON'T:
- Use `z.any()` unless absolutely necessary
- Skip validation constraints
- Duplicate schema definitions

### 4. Error Handling

#### DO:
```typescript
// Define specific error classes
export class TaskNotFoundError extends MoatlessError {
  constructor(taskId: string) {
    super(`Task ${taskId} not found`, ERROR_CODES.TASK_NOT_FOUND, 404)
  }
}

// Use type guards
export function isValidationError(error: ApiResponseError): error is ApiValidationError {
  return error.code === 'VALIDATION_ERROR'
}
```

#### DON'T:
- Throw generic errors
- Use inconsistent error formats
- Skip error details

## Identified Issues and Improvements

### 1. Schema/Type Duplication

**Issue**: Some types are defined both in schemas and types files.

**Solution**:
```typescript
// In schemas file
export const TaskSchema = z.object({ /* ... */ })
export type Task = z.infer<typeof TaskSchema>

// In types index, just re-export
export type { Task } from '../schemas/task.schemas.js'
```

### 2. Inconsistent Response Patterns

**Issue**: Different patterns for paginated responses and list responses.

**Solution**: Standardize on a single pattern:
```typescript
export interface PaginatedResponse<T> {
  items: T[]
  pagination: {
    limit: number
    offset: number
    total: number
    hasMore: boolean
  }
}
```

### 3. Missing Validation in Some Routes

**Issue**: Some routes don't use Zod validation (e.g., settings route uses basic validation).

**Solution**: Use consistent validation across all routes:
```typescript
app.put(
  '/settings',
  describeRoute({ /* ... */ }),
  async (c) => {
    const body = await c.req.json()
    const validated = UpdateSettingsSchema.parse(body)
    // ...
  }
)
```

### 4. Inconsistent Error Response Format

**Issue**: Different error handling patterns across routes.

**Solution**: Create a centralized error middleware:
```typescript
export function createErrorHandler() {
  return async (error: Error, c: Context) => {
    if (error instanceof MoatlessError) {
      return c.json({
        error: {
          code: error.code,
          message: error.message,
          details: error.details
        }
      }, error.statusCode)
    }
    // Handle other errors
  }
}
```

### 5. Mixed Schema Location

**Issue**: Some schemas are defined in route files instead of schema files.

**Solution**: Move all schemas to the shared schemas package:
```typescript
// Move from routes/tasks.ts to schemas/task.schemas.ts
export const FileQuerySchema = z.object({
  path: z.string().optional().default('/'),
  recursive: z.coerce.boolean().optional().default(false),
})
```

## Action Plan

### Phase 1: Schema Consolidation (Priority: High)
1. [ ] Move all inline schemas from route files to shared schemas
2. [ ] Remove duplicate type definitions
3. [ ] Ensure all types are derived from schemas
4. [ ] Add missing schemas for all API endpoints

### Phase 2: Route Standardization (Priority: High)
1. [ ] Create shared error handling middleware
2. [ ] Standardize response formats (especially pagination)
3. [ ] Add consistent OpenAPI documentation to all routes
4. [ ] Extract common route patterns into utilities

### Phase 3: Validation Enhancement (Priority: Medium)
1. [ ] Add Zod validation to all routes
2. [ ] Create custom Zod validators for common patterns
3. [ ] Add request body size limits
4. [ ] Implement rate limiting schemas

### Phase 4: Type Safety Improvements (Priority: Medium)
1. [ ] Remove all `any` types from schemas
2. [ ] Add proper typing to event payloads
3. [ ] Create type guards for all custom types
4. [ ] Add JSDoc comments to exported types

### Phase 5: Documentation (Priority: Low)
1. [ ] Generate OpenAPI spec from routes
2. [ ] Add examples to all schemas
3. [ ] Create API client SDK from types
4. [ ] Document error codes and meanings

## Implementation Guidelines

### For New Features
1. Start with schema definition in `packages/shared/schemas/`
2. Derive types using `z.infer`
3. Implement routes with full validation and error handling
4. Add comprehensive OpenAPI documentation
5. Write tests for both schemas and routes

### For Refactoring
1. Identify and consolidate duplicate definitions
2. Move inline schemas to shared package
3. Update imports to use shared schemas/types
4. Add missing validation and documentation
5. Test thoroughly to ensure no breaking changes

## Code Examples

### Complete Route Pattern
```typescript
// 1. Schema (in shared/schemas/resource.schemas.ts)
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

// 2. Error (in shared/constants/errors.ts)
export class ResourceNotFoundError extends MoatlessError {
  constructor(resourceId: string) {
    super(`Resource ${resourceId} not found`, ERROR_CODES.RESOURCE_NOT_FOUND, 404)
  }
}

// 3. Route (in backend/routes/resources.ts)
export async function registerResourceRoutes(app: Hono<AppBindings>, prefix: string) {
  app.post(
    prefix,
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
        400: {
          description: 'Invalid request',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      
      try {
        const body = await c.req.json()
        const validated = CreateResourceSchema.parse(body)
        
        const resource = await services.resource.create(validated)
        
        return c.json(ResourceResponseSchema.parse(resource), 201)
      } catch (error) {
        handleServiceError(error, 'Create resource')
      }
    },
  )
}
```

This pattern ensures type safety, proper validation, consistent error handling, and good documentation.

## Frontend Patterns and Best Practices

### 1. File Naming Conventions

#### API Files
- **Pattern**: `features/[feature]/api/[feature-name].ts`
- **Examples**: `tasks.ts`, `repositories.ts`, `commands.ts`
- **Exception**: Settings uses `settings.api.ts` suffix

```
✅ features/tasks/api/tasks.ts
✅ features/repositories/api/repositories.ts
❌ features/tasks/api/tasks-api.ts
❌ features/tasks/api/tasksAPI.ts
```

#### Hook Files
- **Pattern**: `features/[feature]/hooks/use-[feature-name].ts`
- **Examples**: `use-tasks.ts`, `use-repositories.ts`, `use-commands.ts`

```
✅ features/tasks/hooks/use-tasks.ts
✅ features/repositories/hooks/use-repositories.ts
❌ features/tasks/hooks/useTasks.ts
❌ features/tasks/hooks/tasks-hooks.ts
```

### 2. API Layer Structure

#### Standard API File Pattern
```typescript
// features/resources/api/resources.ts
import type {
  CreateResourceRequest,
  ResourceResponse,
  ResourceListResponse,
} from '@moatless/shared/types/api'
import { apiClient } from '@/lib/api/client'

export const resourcesApi = {
  async listResources(params?: {
    limit?: number
    offset?: number
  }): Promise<ResourceListResponse> {
    return await apiClient.get<ResourceListResponse>('/api/v1/resources', params)
  },

  async getResource(resourceId: string): Promise<ResourceResponse> {
    return await apiClient.get<ResourceResponse>(`/api/v1/resources/${resourceId}`)
  },

  async createResource(request: CreateResourceRequest): Promise<ResourceResponse> {
    return await apiClient.post<ResourceResponse>('/api/v1/resources', request)
  },

  async updateResource(
    resourceId: string,
    updates: Partial<ResourceResponse>
  ): Promise<ResourceResponse> {
    return await apiClient.put<ResourceResponse>(
      `/api/v1/resources/${resourceId}`,
      updates
    )
  },

  async deleteResource(resourceId: string): Promise<void> {
    return await apiClient.delete<void>(`/api/v1/resources/${resourceId}`)
  },
}
```

### 3. React Query Hook Patterns

#### Query Hook Pattern
```typescript
// features/resources/hooks/use-resources.ts
import type { ResourceListResponse, ResourceResponse } from '@moatless/shared/types/api'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { resourcesApi } from '@/features/resources/api/resources'
import { queryKeys } from '@/lib/api/query-keys'

export function useResourcesList(
  params?: {
    limit?: number
    offset?: number
  },
  options?: Omit<UseQueryOptions<ResourceListResponse>, 'queryKey' | 'queryFn'>
) {
  return useQuery({
    queryKey: queryKeys.resourcesList(params),
    queryFn: () => resourcesApi.listResources(params),
    ...options,
  })
}

export function useResource(
  resourceId: string,
  options?: Omit<UseQueryOptions<ResourceResponse>, 'queryKey' | 'queryFn'>
) {
  return useQuery({
    queryKey: queryKeys.resource(resourceId),
    queryFn: () => resourcesApi.getResource(resourceId),
    enabled: !!resourceId,
    ...options,
  })
}
```

#### Mutation Hook Pattern
```typescript
export function useCreateResource(
  options?: UseMutationOptions<ResourceResponse, Error, CreateResourceRequest>
) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  return useMutation({
    mutationFn: (request: CreateResourceRequest) => resourcesApi.createResource(request),
    onSuccess: async (data, variables, context) => {
      // Invalidate and refetch lists
      await queryClient.invalidateQueries({
        queryKey: queryKeys.resources.lists(),
      })
      
      // Navigate to new resource
      navigate({ to: `/resources/${data.id}` })
      
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useUpdateResource(
  resourceId: string,
  options?: UseMutationOptions<ResourceResponse, Error, Partial<ResourceResponse>>
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationKey: ['resources', resourceId, 'update'],
    mutationFn: (updates) => resourcesApi.updateResource(resourceId, updates),
    onMutate: async (updates) => {
      // Cancel in-flight queries
      await queryClient.cancelQueries({ queryKey: queryKeys.resource(resourceId) })
      
      // Snapshot previous value
      const previousResource = queryClient.getQueryData<ResourceResponse>(
        queryKeys.resource(resourceId)
      )
      
      // Optimistically update
      if (previousResource) {
        queryClient.setQueryData<ResourceResponse>(
          queryKeys.resource(resourceId),
          { ...previousResource, ...updates }
        )
      }
      
      return { previousResource }
    },
    onError: (err, variables, context) => {
      // Rollback on error
      if (context?.previousResource) {
        queryClient.setQueryData(
          queryKeys.resource(resourceId),
          context.previousResource
        )
      }
      options?.onError?.(err, variables, context)
    },
    onSettled: () => {
      // Always refetch after mutation
      queryClient.invalidateQueries({
        queryKey: queryKeys.resource(resourceId),
      })
    },
    ...options,
  })
}
```

### 4. Schema Usage in Frontend

#### Form Validation with Schemas
```typescript
// features/resources/components/resource-form.tsx
import { zodResolver } from '@hookform/resolvers/zod'
import { CreateResourceSchema } from '@moatless/shared/schemas'
import { useForm } from 'react-hook-form'
import type { z } from 'zod'

type FormData = z.infer<typeof CreateResourceSchema>

export function ResourceForm() {
  const form = useForm<FormData>({
    resolver: zodResolver(CreateResourceSchema),
    defaultValues: {
      name: '',
      type: 'default',
    },
  })

  // Form implementation
}
```

#### API Response Validation (Optional)
```typescript
// For critical endpoints, validate responses
import { ResourceResponseSchema } from '@moatless/shared/schemas'

export const resourcesApi = {
  async getResource(resourceId: string): Promise<ResourceResponse> {
    const response = await apiClient.get(`/api/v1/resources/${resourceId}`)
    
    // Optional: Validate response in development
    if (process.env.NODE_ENV === 'development') {
      return ResourceResponseSchema.parse(response)
    }
    
    return response as ResourceResponse
  },
}
```

### 5. Type vs Schema Usage Guidelines

#### Use Types for:
- API function parameters and return types
- Component props
- Hook parameters and return types
- General TypeScript interfaces

#### Use Schemas for:
- Form validation with react-hook-form
- Runtime validation of user input
- API response validation (optional, mainly for debugging)
- Data transformation and parsing

### 6. Query Key Management

```typescript
// lib/api/query-keys.ts
export const queryKeys = {
  // Resource queries
  resources: {
    all: ['resources'] as const,
    lists: () => [...queryKeys.resources.all, 'list'] as const,
    list: (params?: unknown) => [...queryKeys.resources.lists(), params] as const,
    details: () => [...queryKeys.resources.all, 'detail'] as const,
    detail: (id: string) => [...queryKeys.resources.details(), id] as const,
  },
}
```

### 7. Error Handling in Frontend

```typescript
// features/resources/hooks/use-resources.ts
import { getErrorMessage } from '@moatless/shared/types/api-errors'

export function useCreateResource() {
  const { toast } = useToast()

  return useMutation({
    mutationFn: resourcesApi.createResource,
    onError: (error) => {
      const message = getErrorMessage(error)
      toast({
        title: 'Failed to create resource',
        description: message,
        variant: 'destructive',
      })
    },
  })
}
```

### 8. Complete Feature Structure

```
features/resources/
├── api/
│   └── resources.ts          # API client functions
├── components/
│   ├── resource-list.tsx     # List component
│   ├── resource-detail.tsx   # Detail view
│   ├── resource-form.tsx     # Create/Edit form
│   └── resource-card.tsx     # Reusable card component
├── hooks/
│   └── use-resources.ts      # React Query hooks
├── utils/
│   └── resource-helpers.ts   # Feature-specific utilities
└── index.ts                  # Public exports
```

### 9. Frontend Best Practices

#### DO:
- Use types from `@moatless/shared/types/*` for API contracts
- Use schemas from `@moatless/shared/schemas/*` for form validation
- Follow consistent naming patterns
- Include proper TypeScript types for all functions
- Use React Query for all API interactions
- Implement optimistic updates for better UX
- Handle loading and error states consistently

#### DON'T:
- Define types locally that should be shared
- Skip error handling in mutations
- Use different naming patterns
- Make direct API calls without React Query
- Import backend route files directly
- Mix concerns between API and UI layers

### 10. Migration Strategy for Existing Code

1. **Phase 1**: Ensure consistent file naming
   - Rename any files not following conventions
   - Update imports accordingly

2. **Phase 2**: Add missing TypeScript types
   - Type all API responses properly
   - Add types to hook parameters

3. **Phase 3**: Implement schema validation for forms
   - Add zodResolver to all forms
   - Use shared schemas for validation

4. **Phase 4**: Standardize error handling
   - Use getErrorMessage helper
   - Implement consistent toast notifications

5. **Phase 5**: Optimize React Query usage
   - Add optimistic updates where appropriate
   - Implement proper cache invalidation