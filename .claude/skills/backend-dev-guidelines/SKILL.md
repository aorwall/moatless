---
name: backend-dev-guidelines
description: Hono + InversifyJS + Drizzle backend patterns with OpenAPI-first contract design, layered architecture, dependency injection, repository pattern, schema validation, Zod, type-safe configuration, error handling, service layer, routes, controllers, API best practices, database access, ORM, migrations, testing, unit tests, integration tests
---

# Backend Development Guidelines

## Purpose
This skill provides comprehensive guidance for developing backend features in the Moatless Vibe application using Hono, InversifyJS, Drizzle ORM, and OpenAPI-first design.

## Technology Stack

### Core Technologies
- **Framework**: Hono (OpenAPI-first)
- **Runtime**: Bun (dev & production)
- **DI Container**: InversifyJS
- **Database**: Drizzle ORM
- **Validation**: Zod schemas
- **API Contract**: `@moatless/api-contract`
- **Testing**: Vitest

### Key Libraries
- `hono` - Web framework with OpenAPI support
- `@hono/zod-openapi` - OpenAPI integration
- `inversify` - Dependency injection
- `drizzle-orm` - Type-safe ORM
- `zod` - Schema validation
- `@moatless/types` - Shared types

## Quick Reference

| Need to... | See |
|------------|-----|
| Create a new API endpoint | [API Contract Pattern](resources/api-contract-pattern.md) |
| Set up dependency injection | [Dependency Injection](resources/dependency-injection.md) |
| Understand layer responsibilities | [Layered Architecture](resources/layered-architecture.md) |
| Create route handlers | [Route Handlers](resources/route-handlers.md) |
| Implement business logic | [Service Layer](resources/service-layer.md) |
| Access database | [Repository Pattern](resources/repository-pattern.md) |
| Manage database migrations | [Database Migrations](resources/database-migrations.md) |
| Validate data | [Schema Validation](resources/schema-validation.md) |
| Handle errors | [Error Handling](resources/error-handling.md) |
| TypeScript patterns | [TypeScript Patterns](resources/typescript-patterns.md) |
| Organize backend features | [Backend Feature Architecture](resources/backend-feature-architecture.md) |
| Implement feature flags | [Feature Flags](resources/feature-flags.md) |
| Write tests | [Testing](resources/testing.md) |
| See complete examples | [Complete Examples](resources/complete-examples.md) |

## Core Principles

### 1. OpenAPI-First Contract Design
Define API contracts before implementation:
```tsx
// packages/api-contract/src/routes/tasks.ts
import { createRoute, z } from '@hono/zod-openapi'

export const routes = {
  tasks: {
    list: createRoute({
      method: 'get',
      path: '/api/v1/tasks',
      responses: {
        200: {
          content: {
            'application/json': {
              schema: TaskArraySchema
            }
          }
        }
      }
    })
  }
}
```

### 2. Layered Architecture
Strict one-way dependency flow:
```
Routes (HTTP layer)
  ↓
Services (Business logic)
  ↓
Repositories (Data access)
```

### 3. Dependency Injection
All services use InversifyJS for DI:
```tsx
import { injectable, inject } from 'inversify'
import { TYPES } from './types'

@injectable()
export class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private taskRepo: ITaskRepository
  ) {}
}
```

### 4. Type Safety First
- Use TypeScript strict mode
- NEVER use `any` type
- Validate all input with Zod schemas
- Use types from `@moatless/types`

### 5. Schema Validation
Validate ALL input at API boundaries:
```tsx
import { z } from 'zod'

const CreateTaskSchema = z.object({
  title: z.string().min(1),
  description: z.string().optional()
})
```

### 6. Comprehensive Testing
- ALWAYS write unit tests for services
- ADD integration tests for critical flows
- Use naming: `*.test.ts` (unit), `*.integration.test.ts` (integration)

## Critical Anti-Patterns to Avoid

### TypeScript
❌ **NEVER** use `any` type
```tsx
// WRONG
function processData(data: any) { ... }

// CORRECT
function processData(data: TaskData) { ... }
```

❌ **NEVER** use `import type`
```tsx
// WRONG
import type { Task } from '@moatless/types'

// CORRECT
import { Task } from '@moatless/types'
```

### Dependency Injection
❌ **NEVER** omit `@inject()` decorator
```tsx
// WRONG
@injectable()
class TaskService {
  constructor(private repo: TaskRepository) {}  // Missing @inject!
}

// CORRECT
@injectable()
class TaskService {
  constructor(
    @inject(TYPES.TaskRepository) private repo: ITaskRepository
  ) {}
}
```

### Architecture
❌ **DON'T** skip layers
```tsx
// WRONG - Route directly accessing repository
app.get('/tasks', async (c) => {
  const tasks = await taskRepo.findAll()  // Skipping service layer!
  return c.json(tasks)
})

// CORRECT - Route → Service → Repository
app.get('/tasks', async (c) => {
  const tasks = await taskService.getAllTasks()
  return c.json(tasks)
})
```

### API Design
❌ **DON'T** hardcode paths
```tsx
// WRONG
fetch('/api/v1/tasks')

// CORRECT
import { buildPath, routes } from '@moatless/api-contract'
const url = buildPath(routes.tasks.list)
fetch(url)
```

❌ **DON'T** skip input validation
```tsx
// WRONG - No validation
app.post('/tasks', async (c) => {
  const body = await c.req.json()  // Unvalidated!
  const task = await taskService.create(body)
  return c.json(task)
})

// CORRECT - Validate with Zod
app.post('/tasks', zValidator('json', CreateTaskSchema), async (c) => {
  const body = c.req.valid('json')  // Type-safe and validated
  const task = await taskService.create(body)
  return c.json(task)
})
```

## Project Structure Overview

```
apps/backend/
├── src/
│   ├── routes/              # HTTP route handlers
│   │   ├── tasks.ts         # Task routes
│   │   └── environments.ts  # Environment routes
│   ├── services/            # Business logic
│   │   ├── task.service.ts
│   │   └── environment.service.ts
│   ├── repositories/        # Data access layer
│   │   ├── task.repository.ts
│   │   └── environment.repository.ts
│   ├── plugins/             # Hono plugins
│   │   └── logger.ts
│   ├── utils/               # Utilities
│   │   ├── error-handler.ts
│   │   └── config.ts
│   ├── container.ts         # InversifyJS container
│   ├── app.ts              # Hono app setup
│   └── index.ts            # Server entry point
└── drizzle/                # Database migrations
    └── 0001_initial.sql

packages/
├── types/                  # @moatless/types
│   └── src/
│       ├── types/          # Shared type definitions
│       ├── schemas/        # Zod schemas
│       └── index.ts
└── api-contract/          # @moatless/api-contract
    └── src/
        ├── routes/         # Route definitions
        └── index.ts
```

## Common Workflows

### Creating a New API Endpoint
1. Define route contract in `@moatless/api-contract`
2. Create Zod schemas in `@moatless/types`
3. Implement repository for data access
4. Implement service for business logic
5. Create route handler
6. Register route in app
7. Write unit tests for service
8. Write integration test for endpoint

### Adding Business Logic
1. Identify which service owns the logic
2. Add method to service class
3. Inject required dependencies
4. Write unit tests (MANDATORY)
5. Use service from route handler

### Adding Database Access
1. Create/update Drizzle schema
2. Generate migration
3. Implement repository interface
4. Add repository method
5. Register repository in DI container
6. Use from service layer

## Navigation Guide

### Essential Resources (Read First)
1. **[API Contract Pattern](resources/api-contract-pattern.md)** - OpenAPI-first design
2. **[Layered Architecture](resources/layered-architecture.md)** - Layer responsibilities
3. **[Dependency Injection](resources/dependency-injection.md)** - InversifyJS setup

### Implementation Guides
4. **[Route Handlers](resources/route-handlers.md)** - Creating HTTP endpoints
5. **[Service Layer](resources/service-layer.md)** - Business logic patterns
6. **[Repository Pattern](resources/repository-pattern.md)** - Database access
7. **[Database Migrations](resources/database-migrations.md)** - Migration best practices

### Data & Validation
8. **[Schema Validation](resources/schema-validation.md)** - Zod patterns
9. **[Error Handling](resources/error-handling.md)** - Error management

### Testing & Examples
10. **[Testing](resources/testing.md)** - Unit and integration testing
11. **[Complete Examples](resources/complete-examples.md)** - Full implementations

## Quick Start Checklist

### Before Creating an Endpoint
- [ ] Define route contract in `@moatless/api-contract`
- [ ] Create Zod schemas for request/response
- [ ] Design layer responsibilities (Route → Service → Repository)
- [ ] Plan error handling strategy

### Before Implementing a Service
- [ ] Identify dependencies to inject
- [ ] Add `@injectable()` decorator
- [ ] Use `@inject()` for all constructor parameters
- [ ] Plan for testability (inject interfaces, not concrete classes)

### Before Accessing Database
- [ ] Check if Drizzle schema exists
- [ ] Create migration if needed
- [ ] Implement in repository layer (NOT directly in service!)
- [ ] Use repository interface for type safety

### Before Deploying
- [ ] All services have unit tests
- [ ] Critical flows have integration tests
- [ ] Input validation on all endpoints
- [ ] Error handling implemented
- [ ] Database migrations tested on fresh database
- [ ] Production build tested (`bun run build && bun dist/index.js`)
- [ ] Verify `dist/drizzle/` contains all migration files
- [ ] Run `bun run type:check`

## Getting Help

### When Working On...
- **API Endpoints**: See [API Contract Pattern](resources/api-contract-pattern.md)
- **Business Logic**: See [Service Layer](resources/service-layer.md)
- **Database Access**: See [Repository Pattern](resources/repository-pattern.md)
- **Database Migrations**: See [Database Migrations](resources/database-migrations.md)
- **Validation**: See [Schema Validation](resources/schema-validation.md)
- **DI Setup**: See [Dependency Injection](resources/dependency-injection.md)
- **TypeScript**: See [TypeScript Patterns](resources/typescript-patterns.md)
- **Feature Organization**: See [Backend Feature Architecture](resources/backend-feature-architecture.md)
- **Feature Flags**: See [Feature Flags](resources/feature-flags.md)
- **Testing**: See [Testing](resources/testing.md)
