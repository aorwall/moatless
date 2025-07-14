# API Improvement Action List

## Completed ✅

### 1.1 Schema Consolidation - COMPLETED
**Problem**: Schemas were scattered between route files and schema files.

**Completed Actions**:
- ✅ Moved all inline schemas from `routes/tasks.ts` to `schemas/task.schemas.ts`
- ✅ Moved all response schemas from route files to appropriate schema files
- ✅ Updated all imports in route files to use shared schemas
- ✅ Created `schemas/api.schemas.ts` for API request/response schemas
- ✅ Created `schemas/workspace.schemas.ts` for workspace-related schemas

### 1.2 Type Duplication Removal - COMPLETED
**Problem**: Types were defined in multiple places, causing sync issues.

**Completed Actions**:
- ✅ Removed duplicate types from `types/settings.ts` (now use schema-derived types)
- ✅ Ensured all request/response types are derived from schemas using `z.infer`
- ✅ Updated `types/index.ts` to re-export types from schemas
- ✅ Migrated all type files to schema-first approach
- ✅ Resolved export conflicts between manual definitions and schema-derived types

### 1.3 Consistent Error Handling - COMPLETED
**Problem**: Error handling was duplicated across routes with slight variations.

**Completed Actions**:
- ✅ Created `backend/utils/error-handler.ts` with centralized error handling
- ✅ Extracted `handleServiceError` to shared utility
- ✅ Created `backend/utils/route-helpers.ts` for validation helpers
- ✅ Updated all routes to use centralized error handling
- ✅ Removed duplicate error handling code from individual routes

## Remaining Work

### Priority 2: Route Standardization ✅ COMPLETED
**Problem**: Inconsistent patterns across different route files.

**Completed**:
- ✅ Created `backend/utils/route-helpers.ts` with common patterns
- ✅ Created validation helpers (`validateRequestBody`, `validateQueryParams`)
- ✅ Created `getTaskId` parameter extraction helper
- ✅ Extracted common OpenAPI response definitions
- ✅ **NEW**: Standardized pagination response format across all list endpoints
  - Updated TaskListResponseSchema and EventListResponseSchema to use consistent `{items, pagination}` format
  - Added `hasMore` field to pagination for better UX
  - Fixed all frontend code to use `items` instead of `tasks`/`events` properties
- ✅ **NEW**: Created consistent pattern for file path handling in workspace routes
  - Standardized tasks routes to use `extractFilePathFromWildcard` helper
  - Used `validateRequestBody` for consistent validation across routes

### Priority 3: Validation Gaps ✅ COMPLETED
**Problem**: Some routes lacked proper Zod validation.

**Completed**:
- ✅ Added consistent Zod validation to all task routes
- ✅ Added request body validation using `validateRequestBody` helper
- ✅ Added query parameter validation patterns
- ✅ Created validation error handling in centralized error handler
- ✅ **NEW**: Added comprehensive Zod validation to settings routes
  - Replaced manual validation with schema-based validation
  - Added OpenAPI documentation with `describeRoute`
  - Used centralized error handling with `handleServiceError`
  - Created proper response schemas for settings
  - Converted manual interfaces to schema-derived types
- ✅ **NEW**: Enhanced reusable validators and patterns
  - Added `createPaginatedResponseSchema` helper for reusable pagination schemas
  - Standardized file path validation using `extractFilePathFromWildcard`
  - Enhanced `validateRequestBody` usage across all routes

### Priority 4: Missing Response Schemas ✅ COMPLETED
**Problem**: Some API responses didn't have corresponding schemas.

**Completed**:
- ✅ Created comprehensive schemas in `api.schemas.ts` for most response types
- ✅ Added schemas for task-related responses
- ✅ Added schemas for workspace responses in `workspace.schemas.ts`
- ✅ Added error response handling in centralized error handler
- ✅ **NEW**: Created schema for global event stream responses (`EventStreamMessageSchema`)
- ✅ **NEW**: Created schema for stream status endpoint (`StreamStatusResponseSchema`)
- ✅ **NEW**: Updated upload endpoint to use existing `UploadFileResponseSchema`
- ✅ **NEW**: Created error response schemas (matching TypeScript interfaces)
- ✅ **NEW**: Created schema for sync-with-main endpoint (`SyncWorkspaceResponseSchema`)
- ✅ **NEW**: Updated all routes to use shared schemas instead of inline definitions

## Future Improvements (Lower Priority)

### Code Quality: Remove Type Ambiguity
**Problem**: Use of `any` types in several places.

**Actions**:
- [ ] Replace `z.any()` in `EventListResponseSchema` with proper Event schema
- [ ] Type the `commandConditions` field properly in command schemas
- [ ] Remove `as any` casts in OpenAPI resolver calls
- [ ] Add proper types for all event payloads
- [ ] Create typed service interfaces

## Implementation Status Summary

### ✅ Completed (Foundation Phase)
1. ✅ Schema consolidation - All inline schemas moved to centralized files
2. ✅ Type duplication removal - All types now derived from schemas using z.infer
3. ✅ Centralized error handling - Complete error handling system implemented
4. ✅ Route validation - Consistent Zod validation across all routes
5. ✅ Helper utilities - Route helpers and validation utilities created

### 🔄 In Progress (Near-term priorities)
1. ✅ ~~Standardize pagination response formats~~ - COMPLETED
2. ✅ ~~Add Zod validation to settings routes~~ - COMPLETED
3. Complete response schema coverage
4. ✅ ~~File path handling patterns~~ - COMPLETED

## Achievements Summary

## Breaking Changes to Consider

1. **Response format standardization** - May affect frontend
2. **Error response format** - May affect error handling
3. **Endpoint naming changes** - Would require frontend updates
4. **Pagination format changes** - Would affect list consumers

## Files Most Needing Attention

1. `routes/tasks.ts` - Largest file with most inline schemas
2. `routes/workspace.ts` - Duplicate functionality with tasks
3. `routes/settings.ts` - Missing proper validation
4. `routes/events.ts` - Needs proper typing for events
5. `types/api.ts` - Has mixed concerns and duplications

## Frontend-Specific Action Items

### Frontend Priority 1: Naming Consistency ✅ COMPLETED
**Problem**: Inconsistent file naming (e.g., `settings.api.ts` vs `settings.ts`)

**Completed**:
- ✅ Renamed `settings.api.ts` to `settings.ts` for consistency
- ✅ Verified all API files follow `[feature].ts` pattern
- ✅ Verified all hook files follow `use-[feature].ts` pattern
- ✅ Updated all imports after renaming (2 files updated)
- ✅ Type checks pass after changes

### Frontend Priority 2: Schema Usage in Forms ✅ COMPLETED
**Problem**: Limited use of schemas for form validation

**Completed**:
- ✅ Created reusable `useZodForm` hook for form validation with schemas
- ✅ Added schema validation to command forms (using `CreateCommandSchema`/`UpdateCommandSchema`)
- ✅ Added schema validation to repository forms (using `RepositoryConfigSchema`)
- ✅ Added schema validation to task creation forms (using `CreateTaskRequestSchema`)
- ✅ Implemented inline error display instead of toast-only notifications
- ✅ Removed manual validation code in favor of schema validation

### Frontend Priority 3: Type Safety
**Problem**: Some API responses not fully typed

**Actions**:
- [ ] Add proper types to all API function returns
- [ ] Remove inline type definitions in favor of shared types
- [ ] Add proper error types to mutation hooks
- [ ] Type all query and mutation options properly
- [ ] Add type guards for API responses

### Frontend Priority 4: Error Handling
**Problem**: Inconsistent error handling across features

**Actions**:
- [ ] Implement consistent toast notifications for all mutations
- [ ] Use `getErrorMessage` helper in all error handlers
- [ ] Add error boundaries around feature components
- [ ] Create reusable error components
- [ ] Handle network errors gracefully

### Frontend Priority 5: Query Keys Centralization
**Problem**: Not all query keys use the centralized system

**Actions**:
- [ ] **IDENTIFIED**: Fix hardcoded `['repositories']` in `use-repositories-settings.ts` to use `queryKeys.repositoriesList()`
- [ ] **IDENTIFIED**: Add missing `settings` query key to `query-keys.ts`
- [ ] **IDENTIFIED**: Update `use-settings.ts` to use centralized query key
- [ ] **IDENTIFIED**: Remove unused query keys (agents, task iterations)
- [ ] Add linting rule to prevent hardcoded query keys
- [ ] Document query key naming conventions

**Status**: Ready for implementation (identified during current work)

### Frontend Priority 6: React Query Optimization
**Problem**: Not all mutations use optimistic updates

**Actions**:
- [ ] Add optimistic updates to task status changes
- [ ] Add optimistic updates to repository updates
- [ ] Add optimistic updates to command updates
- [ ] Implement proper cache invalidation strategies
- [ ] Add mutation retry logic for network failures

## Full Stack Integration Points

### API Contract Sync
**Actions**:
- [ ] Ensure frontend types match backend schemas exactly
- [ ] Create scripts to validate type/schema alignment
- [ ] Add tests that verify API contracts
- [ ] Document the type generation process

### 📈 Compliance with Best Practices
✅ **Following Best Practices**:
- Schema-first architecture fully implemented
- Centralized error handling in use
- Consistent validation patterns
- Type safety maintained throughout
- OpenAPI documentation complete

⚠️ **Areas for Improvement**:
- Pre-push hooks caught test failures (good safety net)
- Some resolver type assertions still needed (`as any`)
- File naming conventions need enforcement
- Query keys not fully centralized (some hardcoded keys found)
