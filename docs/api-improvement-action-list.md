# API Improvement Action List

## Priority 1: Critical Issues (Do First)

### 1.1 Schema Consolidation
**Problem**: Schemas are scattered between route files and schema files, causing maintenance issues.

**Actions**:
- [ ] Move `FileQuerySchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move `FileContentSchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move `FileSearchSchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move `CommandExecutionSchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move `ServerLogsQuerySchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move `MergeTaskSchema` from `routes/tasks.ts` to `schemas/task.schemas.ts`
- [ ] Move all response schemas from route files to appropriate schema files
- [ ] Update all imports in route files to use shared schemas

### 1.2 Type Duplication Removal
**Problem**: Types are defined in multiple places, causing sync issues.

**Actions**:
- [ ] Remove `UpdateSettingsRequest` from `types/settings.ts` (use schema-derived type)
- [ ] Remove duplicate pagination types and consolidate in one place
- [ ] Ensure all request/response types are derived from schemas using `z.infer`
- [ ] Update `types/index.ts` to re-export types from schemas

### 1.3 Consistent Error Handling
**Problem**: Error handling is duplicated across routes with slight variations.

**Actions**:
- [ ] Create `backend/utils/error-handler.ts` with centralized error handling
- [ ] Extract `handleServiceError` to shared utility
- [ ] Create consistent error response format middleware
- [ ] Update all routes to use centralized error handling
- [ ] Remove duplicate error handling code from individual routes

## Priority 2: Important Improvements

### 2.1 Route Standardization
**Problem**: Inconsistent patterns across different route files.

**Actions**:
- [ ] Create `backend/utils/route-helpers.ts` with common patterns
- [ ] Standardize parameter extraction (create `getRequiredParam` helper)
- [ ] Standardize pagination response format across all list endpoints
- [ ] Create consistent pattern for file path handling in workspace routes
- [ ] Extract common OpenAPI response definitions

### 2.2 Validation Gaps
**Problem**: Some routes lack proper Zod validation.

**Actions**:
- [ ] Add Zod validation to settings routes (replace manual validation)
- [ ] Add request body validation to all POST/PUT endpoints
- [ ] Add query parameter validation to all GET endpoints with params
- [ ] Create reusable validators for common patterns (IDs, paths, etc.)
- [ ] Add validation error details to error responses

### 2.3 Missing Response Schemas
**Problem**: Some API responses don't have corresponding schemas.

**Actions**:
- [ ] Create schema for global event stream responses
- [ ] Create schema for upload endpoint responses
- [ ] Create schema for workspace status responses
- [ ] Add schemas for all error response types
- [ ] Ensure all endpoints have typed responses

## Priority 3: Code Quality Improvements

### 3.1 Remove Type Ambiguity
**Problem**: Use of `any` types in several places.

**Actions**:
- [ ] Replace `z.any()` in `EventListResponseSchema` with proper Event schema
- [ ] Type the `commandConditions` field properly in command schemas
- [ ] Remove `as any` casts in OpenAPI resolver calls
- [ ] Add proper types for all event payloads
- [ ] Create typed service interfaces

### 3.2 API Consistency
**Problem**: Inconsistent API patterns and naming.

**Actions**:
- [ ] Standardize endpoint naming (e.g., `/search-files` vs `/files/search`)
- [ ] Consistent use of HTTP methods (POST for actions vs GET)
- [ ] Standardize query parameter names across similar endpoints
- [ ] Align response property names (e.g., `items` vs specific names)
- [ ] Create naming convention guide

### 3.3 OpenAPI Documentation
**Problem**: Incomplete or missing OpenAPI documentation.

**Actions**:
- [ ] Add examples to all OpenAPI schemas
- [ ] Document all possible error responses
- [ ] Add descriptions to all schema properties
- [ ] Create reusable OpenAPI components
- [ ] Generate OpenAPI spec file for external consumption

## Priority 4: Architecture Improvements

### 4.1 Service Layer Abstraction
**Problem**: Routes directly handle too much business logic.

**Actions**:
- [ ] Create clear service interfaces with TypeScript types
- [ ] Move business logic from routes to services
- [ ] Add proper service error types
- [ ] Create service response types separate from API responses
- [ ] Implement service-level validation

### 4.2 Middleware Enhancement
**Problem**: Missing common middleware for cross-cutting concerns.

**Actions**:
- [ ] Create request ID middleware for tracing
- [ ] Add request/response logging middleware
- [ ] Implement request body size limit middleware
- [ ] Add request timeout middleware
- [ ] Create API versioning middleware

### 4.3 Testing Infrastructure
**Problem**: Testing could be improved with better type safety.

**Actions**:
- [ ] Create type-safe test helpers for routes
- [ ] Add schema validation tests
- [ ] Create fixtures using schemas
- [ ] Add API contract tests
- [ ] Implement response schema validation in tests

## Implementation Order

### Week 1-2: Foundation
1. Schema consolidation (1.1)
2. Type duplication removal (1.2)
3. Centralized error handling (1.3)

### Week 3-4: Standardization
1. Route standardization (2.1)
2. Validation gaps (2.2)
3. Missing response schemas (2.3)

### Week 5-6: Quality
1. Remove type ambiguity (3.1)
2. API consistency (3.2)
3. OpenAPI documentation (3.3)

### Week 7-8: Architecture
1. Service layer improvements (4.1)
2. Middleware enhancement (4.2)
3. Testing infrastructure (4.3)

## Quick Wins (Can be done immediately)

1. **Move inline schemas** - Simple cut and paste operation
2. **Remove duplicate types** - Delete redundant definitions
3. **Add missing validation** - Add `.parse()` calls to routes
4. **Fix `any` types** - Replace with proper types where obvious
5. **Standardize error messages** - Update to consistent format

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

## Metrics for Success

- [ ] Zero inline schemas in route files
- [ ] Zero duplicate type definitions
- [ ] 100% routes with Zod validation
- [ ] Zero `any` types in schemas
- [ ] All routes with OpenAPI documentation
- [ ] Centralized error handling in use
- [ ] Consistent response formats
- [ ] Generated OpenAPI spec available

## Frontend-Specific Action Items

### Frontend Priority 1: Naming Consistency
**Problem**: Inconsistent file naming (e.g., `settings.api.ts` vs `settings.ts`)

**Actions**:
- [ ] Rename `settings.api.ts` to `settings.ts` for consistency
- [ ] Ensure all API files follow `[feature].ts` pattern
- [ ] Ensure all hook files follow `use-[feature].ts` pattern
- [ ] Update all imports after renaming

### Frontend Priority 2: Schema Usage in Forms
**Problem**: Limited use of schemas for form validation

**Actions**:
- [ ] Add schema validation to task creation forms
- [ ] Add schema validation to command forms
- [ ] Add schema validation to settings forms
- [ ] Create reusable form components that use schemas
- [ ] Document form validation patterns

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

### Frontend Priority 5: React Query Optimization
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
- [ ] Set up automated type checking in CI

### Development Workflow
**Actions**:
- [ ] Create a script to generate types from schemas
- [ ] Add pre-commit hooks to validate types
- [ ] Create development guidelines for adding new features
- [ ] Document the shared package usage
- [ ] Set up type checking across packages

## Notes

- Start with non-breaking changes first
- Test thoroughly after each change
- Update frontend types after backend changes
- Consider creating a migration guide for breaking changes
- Set up linting rules to enforce patterns going forward
- Prioritize user-facing improvements (optimistic updates, error handling)
- Consider adding E2E tests for critical API flows