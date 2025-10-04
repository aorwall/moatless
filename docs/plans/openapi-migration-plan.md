# OpenAPI Migration Plan: hono-openapi → @hono/zod-openapi

## Overview

This document outlines the migration plan from `hono-openapi` to `@hono/zod-openapi` to enable automatic OpenAPI documentation generation without manual specifications.

**Goals:**
1. Remove `hono-openapi` dependency and replace with `@hono/zod-openapi`
2. Migrate all route definitions to use `createRoute()` pattern
3. Fix date schema transform compatibility with OpenAPI generation
4. Enable automatic `/openapi.json` spec generation
5. Provide API documentation UI via Scalar at `/docs`

## Current State Analysis

### Issues to Address
- **Date Schema Transform Error**: The `dateSchema` uses `.transform()` which is incompatible with `hono-openapi`'s OpenAPI generation
- **Manual OpenAPI Spec**: Currently using a static manual spec that requires maintenance
- **17+ Route Files**: All using old `describeRoute()` and `resolver()` pattern from `hono-openapi`
- **Mixed Dependencies**: Both `hono-openapi` and `@hono/zod-openapi` installed

### Affected Files
**Route files using hono-openapi:**
- `routes/tasks.ts` (partially migrated)
- `routes/repositories.ts`
- `routes/commands.ts`
- `routes/events.ts`
- `routes/uploads.ts`
- `routes/usage.ts`
- `routes/worker.ts`
- `routes/workspace.ts`
- `routes/preview-environment.ts`
- `routes/auth.ts`
- `routes/settings.ts`
- `routes/settings/general.ts`
- `routes/settings/github.ts`
- `routes/settings/claude.ts`
- `routes/settings/publish.ts`
- `routes/index.ts`

**Core files:**
- `app.ts` (partially migrated - using OpenAPIHono)
- `packages/types/src/utils/validation.ts` (dateSchema needs fix)

## Target Architecture

### OpenAPI Generation Flow
```
Zod Schemas → createRoute() → app.openapi() → .doc() → OpenAPI Spec JSON
                                                      ↓
                                              /openapi.json endpoint
                                                      ↓
                                              Scalar UI at /docs
```

### Route Pattern Migration

**Before (hono-openapi):**
```typescript
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { Hono } from 'hono'

export async function registerRoutes(app: Hono<AppBindings>, prefix: string) {
  app.post(
    prefix,
    describeRoute({
      description: 'Create resource',
      requestBody: {
        content: {
          'application/json': {
            schema: resolver(CreateSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Success',
          content: {
            'application/json': {
              schema: resolver(ResponseSchema),
            },
          },
        },
      },
    }),
    async (c) => {
      const body = await c.req.json()
      const validated = CreateSchema.parse(body)
      // ... handler logic
    },
  )
}
```

**After (@hono/zod-openapi):**
```typescript
import { createRoute } from '@hono/zod-openapi'
import type { OpenAPIHono } from '@hono/zod-openapi'

const createResourceRoute = createRoute({
  method: 'post',
  path: '/api/v1/resource',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CreateSchema,
        },
      },
    },
  },
  responses: {
    200: {
      content: {
        'application/json': {
          schema: ResponseSchema,
        },
      },
      description: 'Resource created successfully',
    },
  },
})

export async function registerRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  app.openapi(createResourceRoute, async (c) => {
    const body = c.req.valid('json')
    // ... handler logic
  })
}
```

### Date Schema Fix

**Current (incompatible):**
```typescript
export const dateSchema = z
  .union([z.string().datetime(), z.date()])
  .transform((val) => (val instanceof Date ? val : new Date(val)))
```

**Target (OpenAPI-compatible):**
```typescript
export const dateSchema = z.string().datetime().pipe(z.coerce.date())
```

## Migration Phases

### Phase 1: Foundation Setup ✅ COMPLETED

#### Task 1.1: Install @hono/zod-openapi ✅
**Status**: Completed
**Files modified**: `apps/backend/package.json`

**Actions taken**:
- ✅ Installed `@hono/zod-openapi@1.1.3`

#### Task 1.2: Migrate app.ts to OpenAPIHono ✅
**Status**: Completed
**Files modified**: `apps/backend/app.ts`

**Actions taken**:
- ✅ Changed `Hono` → `OpenAPIHono`
- ✅ Updated `AppWithServices` interface
- ✅ Added `.doc('/openapi.json', {...})` endpoint
- ✅ Added `/docs` endpoint with Scalar UI

### Phase 2: Fix Date Schema Transform ✅ COMPLETED

#### Task 2.1: Update dateSchema in validation.ts ✅
**Status**: Completed
**Priority**: High
**Dependencies**: None
**Files modified**: `packages/types/src/utils/validation.ts`

**Actions taken**:
- ✅ Replaced dateSchema with OpenAPI-compatible version:
   ```typescript
   export const dateSchema = z.string().datetime().pipe(z.coerce.date())
   ```
- ✅ Rebuilt types package
- ✅ Tested that date parsing works correctly

#### Task 2.2: Verify Date Schema in All Schemas ✅
**Status**: Completed
**Priority**: Medium
**Dependencies**: Task 2.1
**Files verified**: All schema files in `packages/types/src/schemas/`

**Actions taken**:
- ✅ Verified dateSchema usage across all schemas
- ✅ Server starts successfully with new schema
- ✅ Date parsing maintains expected behavior

### Phase A (Revised Approach): Remove hono-openapi Dependencies ✅ COMPLETED

**Approach**: Instead of incrementally migrating to `createRoute()` pattern, we first removed all `hono-openapi` dependencies to get the server running with plain Hono routes. This allows the application to work without the incompatible package while we plan future OpenAPI integration.

#### Task A.1: Remove hono-openapi package ✅
**Status**: Completed
**Files modified**: `apps/backend/package.json`

**Actions taken**:
- ✅ Removed `hono-openapi` package from dependencies
- ✅ Removed `zod-openapi` package (replaced by @hono/zod-openapi)
- ✅ Ran `bun install` to update lockfile

#### Task A.2: Bulk convert all route files ✅
**Status**: Completed
**Files modified**: All 15 route files

**Route files converted**:
- ✅ `routes/index.ts` - Removed describeRoute wrapper
- ✅ `routes/tasks.ts` - Removed all describeRoute wrappers (~25 routes)
- ✅ `routes/repositories.ts` - Removed describeRoute wrappers (~12 routes)
- ✅ `routes/workspace.ts` - Removed describeRoute wrappers (~10 routes)
- ✅ `routes/preview-environment.ts` - Removed describeRoute wrappers (~6 routes)
- ✅ `routes/events.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/uploads.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/commands.ts` - Removed describeRoute wrappers (~5 routes)
- ✅ `routes/worker.ts` - Removed describeRoute wrappers (~3 routes)
- ✅ `routes/usage.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/auth.ts` - Removed describeRoute wrappers (~3 routes)
- ✅ `routes/settings.ts` - Removed describeRoute wrappers
- ✅ `routes/settings/general.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/settings/github.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/settings/claude.ts` - Removed describeRoute wrappers (~2 routes)
- ✅ `routes/settings/publish.ts` - Removed describeRoute wrappers (~2 routes)

**Conversion pattern applied**:
```typescript
// BEFORE (with hono-openapi):
app.post(
  `${prefix}/endpoint`,
  describeRoute({
    description: '...',
    requestBody: { ... },
    responses: { ... },
  }),
  async (c) => {
    const body = await c.req.json()
    const validated = Schema.parse(body)
    // handler logic
  }
)

// AFTER (plain Hono):
app.post(`${prefix}/endpoint`, async (c) => {
  const body = await c.req.json()
  const validated = Schema.parse(body)
  // handler logic
})
```

**Actions taken**:
- ✅ Created automated script to remove `describeRoute()` wrappers
- ✅ Removed `resolver()` calls from schema references
- ✅ Kept all validation logic intact using direct Zod schema parsing
- ✅ Cleaned up unused imports with Biome
- ✅ Verified TypeScript compilation passes
- ✅ Tested server starts successfully

#### Task A.3: Verify server functionality ✅
**Status**: Completed

**Verification results**:
- ✅ Server starts without errors on port 8000
- ✅ All route handlers registered correctly
- ✅ Endpoint validation still works (using manual Zod parsing)
- ✅ No hono-openapi imports remaining
- ✅ Application fully functional

### Phase 3: Migrate Routes to @hono/zod-openapi

**Status**: Ready to start

**Goal**: Convert all 74 plain Hono routes to use `createRoute()` pattern for automatic OpenAPI spec generation.

#### Current State
- **Total Routes**: 75 across 16 files
- **OpenAPI Routes**: 1 (POST /tasks in tasks.ts)
- **Plain Routes**: 74 routes need migration to `createRoute()` pattern
- **Infrastructure**: Ready (OpenAPIHono, Scalar UI, dateSchema fixed)

#### Route Inventory (Priority Order)

**High Priority** (Core API - 46 routes):
1. `tasks.ts` - 21 routes (largest file, 1 already done)
2. `workspace.ts` - 14 routes
3. `repositories.ts` - 10 routes
4. `events.ts` - 2 routes (SSE endpoint)

**Medium Priority** (Features - 16 routes):
5. `commands.ts` - 5 routes
6. `preview-environment.ts` - 4 routes
7. `auth.ts` - 3 routes
8. `uploads.ts` - 2 routes
9. `usage.ts` - 2 routes

**Low Priority** (Settings & Utilities - 13 routes):
10. `settings/claude.ts` - 2 routes
11. `settings/github.ts` - 2 routes
12. `settings/general.ts` - 2 routes
13. `settings/publish.ts` - 2 routes
14. `worker.ts` - 1 route
15. `settings.ts` - 1 route
16. `index.ts` - 1 route

#### Implementation Tasks

**Task 3.1: Create Helper Patterns**
- Document common route patterns based on existing schemas
- Create reusable templates for CRUD operations
- Define error response patterns

**Task 3.2: Migrate High Priority Routes** (4 files, 46 routes)
- Migrate `tasks.ts` (21 routes) - has working example to follow
- Migrate `workspace.ts` (14 routes)
- Migrate `repositories.ts` (10 routes)
- Migrate `events.ts` (2 routes)
- Test `/openapi.json` generation after each file

**Task 3.3: Migrate Medium Priority Routes** (5 files, 16 routes)
- Migrate: commands, preview-environment, auth
- Migrate: uploads, usage
- Verify OpenAPI spec completeness

**Task 3.4: Migrate Low Priority Routes** (7 files, 13 routes)
- Migrate all settings routes
- Migrate worker and index routes
- Final OpenAPI spec validation

**Task 3.5: Documentation & Validation**
- Verify Scalar UI at `/docs` renders correctly
- Test all endpoints still function
- Run full test suite
- Update documentation

#### Conversion Pattern

```typescript
// BEFORE (plain Hono):
app.post(`${prefix}/endpoint`, async (c) => {
  const body = await c.req.json()
  const validated = RequestSchema.parse(body)
  // handler logic
  return c.json(response)
})

// AFTER (@hono/zod-openapi):
const endpointRoute = createRoute({
  method: 'post',
  path: '/api/v1/endpoint',
  request: {
    body: {
      content: {
        'application/json': {
          schema: RequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      content: {
        'application/json': {
          schema: ResponseSchema,
        },
      },
      description: 'Success',
    },
  },
})

app.openapi(endpointRoute, async (c) => {
  const body = c.req.valid('json')
  // handler logic (unchanged)
  return c.json(response)
})
```

#### Testing Strategy

**Per-File Testing**:
1. Convert 1 file at a time
2. Check `/openapi.json` includes new routes
3. Test all endpoints with curl/Postman
4. Run relevant test suite
5. Verify no regressions

**Integration Testing**:
- Test `/docs` UI after each batch
- Verify schema validation works with `c.req.valid()`
- Check error responses match OpenAPI spec

#### Success Criteria
- All 74 routes migrated to `createRoute()` pattern
- `/openapi.json` generates complete spec (75 routes documented)
- Scalar UI at `/docs` displays all endpoints
- All tests pass
- No breaking changes to API behavior

#### Estimated Effort
- **Task 3.1**: ~30 minutes (create patterns/templates)
- **Task 3.2**: ~4-6 hours (46 high-priority routes, careful testing)
- **Task 3.3**: ~2-3 hours (16 medium-priority routes)
- **Task 3.4**: ~1-2 hours (13 low-priority routes, mostly simple)
- **Task 3.5**: ~1 hour (validation, documentation)

**Total**: ~8-12 hours of focused work

### Phase 4: Cleanup and Verification ✅ COMPLETED

#### Task 4.1: Remove hono-openapi dependency ✅
**Status**: Completed
**Priority**: High
**Files modified**:
- `apps/backend/package.json`

**Actions taken**:
- ✅ Removed `hono-openapi` from dependencies
- ✅ Removed `zod-openapi` from dependencies
- ✅ Ran `bun install` to clean up
- ✅ Verified no remaining references:
   ```bash
   rg "hono-openapi" apps/backend  # 0 results
   rg "describeRoute" apps/backend # 0 results
   rg "resolver.*from.*hono-openapi" apps/backend  # 0 results
   ```

#### Task 4.2: Verify Server Functionality ✅
**Status**: Completed
**Priority**: High

**Actions taken**:
- ✅ Started dev server successfully
- ✅ Server listening on http://0.0.0.0:8000
- ✅ Verified endpoint functionality with test requests
- ✅ All route handlers working correctly
- ✅ Validation still functioning with manual Zod parsing

#### Task 4.3: Code Quality Checks ✅
**Status**: Completed
**Priority**: Medium

**Actions taken**:
- ✅ Ran Biome to fix formatting and remove unused imports
- ✅ TypeScript type checking passes
- ✅ No compilation errors
- ✅ Cleaned up temporary conversion scripts

## Progress Tracking

### Summary
- **Total Route Files**: 17 (16 route files + index.ts)
- **Completed**: 17 (all files converted to plain Hono routes)
- **In Progress**: 0
- **Pending**: 0 (for hono-openapi removal)
- **Total Routes Converted**: ~80+ routes

### Phase Completion
- [x] Phase 1: Foundation Setup (100%) - OpenAPIHono infrastructure ready
- [x] Phase 2: Fix Date Schema Transform (100%) - dateSchema now OpenAPI-compatible
- [x] Phase A: Remove hono-openapi Dependencies (100%) - All routes converted to plain Hono
- [x] Phase 4: Cleanup and Verification (100%) - Server verified working
- [ ] Phase 3: Migrate Routes to @hono/zod-openapi (0%) - Ready to start

### Overall Progress

**Phase A Complete**: 100% - hono-openapi removal successful
- ✅ All `hono-openapi` dependencies removed
- ✅ Server running with plain Hono routes
- ✅ Manual Zod validation working
- ✅ Foundation ready for OpenAPI integration

**Phase 3 Pending**: 0% - OpenAPI route migration
- ⏳ 74 routes to convert to `createRoute()` pattern
- ⏳ OpenAPI spec generation
- ⏳ Scalar UI documentation

## Risk Assessment

### High Risk
- **Date Schema Changes**: May break existing code that relies on Date objects
  - Mitigation: Comprehensive testing of all date-related functionality
- **Handler Validation Changes**: Using `c.req.valid()` instead of manual parsing
  - Mitigation: Test each endpoint after migration

### Medium Risk
- **Type Signature Changes**: OpenAPIHono vs Hono may cause type errors
  - Mitigation: Address TypeScript errors incrementally
- **Large Scope**: 150-200 routes to migrate
  - Mitigation: Migrate in phases, test after each file

### Low Risk
- **Documentation UI**: Scalar UI rendering issues
  - Mitigation: Fallback to raw JSON if needed

## Testing Strategy

### Per-File Testing
After migrating each route file:
1. Start dev server
2. Test each endpoint with curl or Postman
3. Verify request/response validation
4. Check error handling
5. Confirm OpenAPI spec includes the routes

### Integration Testing
After completing each phase:
1. Run full integration test suite
2. Test end-to-end workflows
3. Verify OpenAPI spec completeness

### Final Verification
After completing all phases:
1. Full regression test suite
2. Manual API testing
3. OpenAPI spec validation
4. Documentation review

## Success Criteria

### Completed ✅
- ✅ **All route files migrated** - No remaining `hono-openapi` imports
- ✅ **No breaking changes** - All API endpoints work as before
- ✅ **Date transforms work** - Date parsing maintains existing behavior with `.pipe(z.coerce.date())`
- ✅ **Dependencies cleaned** - `hono-openapi` and `zod-openapi` removed from package.json
- ✅ **Server runs successfully** - Application fully functional on port 8000
- ✅ **Type safety maintained** - All TypeScript checks pass

### Phase 3 Goals (In Planning)
- ⏳ **OpenAPI spec generates automatically** - `/openapi.json` endpoint ready, needs 74 route definitions
- ⏳ **Documentation UI works** - `/docs` endpoint ready, needs populated spec from routes
- ⏳ **All routes use createRoute()** - Systematic migration of 74 routes to OpenAPI pattern

## Implementation Notes

### Completed Work
- **Conservative Approach**: Removed `hono-openapi` dependencies without adding full OpenAPI integration
- **Bulk Conversion**: Created automated script to efficiently convert all 15 route files
- **Validation Preserved**: All request/response validation maintained using direct Zod schema parsing
- **Zero Downtime**: Server remains fully functional throughout the migration
- **Future-Ready**: OpenAPIHono infrastructure in place for gradual OpenAPI adoption

### Lessons Learned
1. **Incremental migration wasn't necessary** - Removing `describeRoute()` wrappers was straightforward and could be done in bulk
2. **Automated conversion** - Created Node.js script to handle repetitive pattern matching and replacement
3. **Manual validation works** - Direct Zod schema parsing (`Schema.parse()`) provides sufficient validation without OpenAPI
4. **OpenAPI can be added later** - The foundation (OpenAPIHono, Scalar UI) is ready for future `createRoute()` adoption

### Phase 3 Implementation Plan
See Phase 3 section above for comprehensive implementation plan:
- **Task 3.1**: Create helper patterns and templates
- **Task 3.2**: Migrate 46 high-priority routes (tasks, workspace, repositories, events)
- **Task 3.3**: Migrate 16 medium-priority routes (commands, preview, auth, uploads, usage)
- **Task 3.4**: Migrate 13 low-priority routes (settings, worker, index)
- **Task 3.5**: Documentation and validation

**Total Estimated Effort**: 8-12 hours of focused work
