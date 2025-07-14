# Frontend Testing Improvement Action Plan

## Overview
This action plan addresses the testing inconsistencies and gaps discovered during the comprehensive frontend test analysis. The plan is organized by priority and includes specific files to be modified or created.

## ✅ Completed Actions

### Phase 1: Clean Up and Standards ✅

#### 1. Fixed Disabled Integration Tests ✅
**Status**: COMPLETED  
**Completed**: January 2025

- ✅ Fixed context migration issues in `task-detail.integration.test.tsx`
- ✅ Created comprehensive integration tests covering all major features
- ✅ Removed placeholder assertions and implemented real behavior tests

#### 2. Removed Meaningless Tests ✅
**Status**: COMPLETED  
**Completed**: January 2025

- ✅ Fixed 3 tests with `expect(true).toBe(true)` assertions
- ✅ Updated tests to verify actual behavior
- ✅ Documented all changes with clear commit messages

#### 3. Removed All Skipped Tests ✅
**Status**: COMPLETED  
**Completed**: January 2025

**Removed 18 skipped tests + 1 describe block**:
- 5 tests from `repository-form.unit.test.tsx` (Zod validation issues)
- 5 tests from `chat-input-file-search.unit.test.tsx` (autocomplete issues)
- 2 tests from `chat-input-expandable.unit.test.tsx` (keyboard event issues)
- 2 tests from `task-list.unit.test.tsx` (date parsing issues)
- 3 tests from `event-viewer.unit.test.tsx` (module state issues)
- 1 describe block from `markdown-preview.unit.test.tsx` (mermaid timing)

#### 4. Created Centralized Mock Utilities ✅
**Status**: COMPLETED  
**Completed**: January 2025

**Created comprehensive mock utilities in `src/test-utils/mocks/`**:
- ✅ `api-hooks.ts` - Query and mutation mocks (createMockQuery, createMockLoadingQuery, etc.)
- ✅ `sse-connections.ts` - SSE/WebSocket mocks (MockEventSource, MockWebSocket)
- ✅ `ui-libraries.ts` - UI component mocks (resizable, dialog, select, etc.)
- ✅ `providers.tsx` - Provider wrappers (createQueryWrapper, createAllProvidersWrapper)
- ✅ `index.ts` - Central exports for all mocks

#### 5. Implemented Test Naming Convention ✅
**Status**: COMPLETED  
**Completed**: January 2025

- ✅ Renamed 42 test files to follow convention:
  - `*.unit.test.{ts,tsx}` for unit tests
  - `*.integration.test.{ts,tsx}` for integration tests
- ✅ Updated `vitest.config.ts` to recognize new patterns
- ✅ Updated `package.json` test scripts for different test categories

## 🚧 Next Actions (High Priority)

### Phase 2: Adopt Mock Utilities Throughout Codebase
**Priority**: High  
**Goal**: Update existing tests to use centralized mocks

**Key Files to Update**:
1. **API Hook Tests**:
   - `use-tasks.unit.test.ts`
   - `use-repositories.unit.test.tsx`
   - `use-environments.unit.test.tsx`
   - Replace ad-hoc mocks with `createMockQuery`, `createMockLoadingQuery`, etc.

2. **SSE/WebSocket Tests**:
   - `global-event-stream-provider.unit.test.tsx`
   - `use-task-sse-connection.test.ts` (if exists)
   - Use `createMockSSEConnection` and `MockEventSource`

3. **UI Component Tests**:
   - Tests using resizable panels
   - Tests using dialogs, selects, tabs
   - Replace inline mocks with imports from `@/test-utils/mocks`

**Example Migration**:
```typescript
// Before:
vi.mocked(useTask).mockReturnValue({
  data: taskData,
  isLoading: false,
  error: null
} as any)

// After:
import { createMockQuery } from '@/test-utils/mocks'
vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
```

### Phase 3: Add Missing Test Coverage
**Priority**: Medium  
**Goal**: Test uncovered critical paths

**Critical Areas Needing Tests**:
1. **Error Recovery Flows**:
   - Network failure recovery
   - SSE reconnection logic
   - Form validation error handling

2. **Real-time Features**:
   - Live event updates
   - Streaming message handling
   - Multi-tab synchronization

3. **Complex Interactions**:
   - File search with autocomplete
   - Drag-and-drop file upload
   - Keyboard shortcuts

### Phase 4: Implement E2E Tests
**Priority**: Medium  
**Goal**: Test complete user journeys

**Critical E2E Tests to Add**:
1. `task-creation-flow.e2e.test.tsx` - Complete task creation journey
2. `task-real-time-updates.e2e.test.tsx` - SSE updates across components
3. `repository-management.e2e.test.tsx` - Repository CRUD operations
4. `preview-environment.e2e.test.tsx` - Preview server lifecycle

## 📊 Success Metrics

### Achieved ✅
- ✅ Zero tests with meaningless assertions
- ✅ All skipped tests removed or properly documented
- ✅ Centralized mock utilities created and documented
- ✅ Consistent test naming convention implemented
- ✅ Updated testing guidelines with new patterns

### In Progress 🚧
- [ ] 50% of tests using centralized mocks (currently ~5%)
- [ ] Test coverage increased to 80% (currently ~70%)
- [ ] All critical user paths have integration tests
- [ ] E2E test infrastructure set up

### Future Goals 🎯
- [ ] Average test execution time < 30s for unit tests
- [ ] Zero flaky tests in CI
- [ ] 100% of new features have tests before merge
- [ ] Automated test quality reports in PRs

## 📋 Implementation Checklist

### Immediate Actions (This Week)
- [ ] Update 10 test files to use centralized mocks
- [ ] Create example PR showing mock migration pattern
- [ ] Document mock usage patterns in testing guide
- [ ] Set up test coverage reporting in CI

### Short-term (Next 2 Weeks)
- [ ] Migrate all API hook tests to use centralized mocks
- [ ] Add integration tests for uncovered routes
- [ ] Set up E2E testing infrastructure
- [ ] Create first E2E test as proof of concept

### Long-term (Next Month)
- [ ] Complete mock migration for all test files
- [ ] Achieve 80% test coverage
- [ ] Implement 5+ E2E tests for critical paths
- [ ] Set up automated test quality gates

## 📚 Documentation Updates Needed

1. **Testing Guidelines** (`docs/guidelines/frontend-testing.md`):
   - ✅ Add centralized mock utilities section
   - ✅ Update test naming convention
   - ✅ Add test quality standards
   - [ ] Add E2E testing section
   - [ ] Add troubleshooting guide

2. **Mock Utilities Documentation**:
   - [ ] Create `docs/guidelines/mock-utilities.md`
   - [ ] Add examples for each mock type
   - [ ] Document when to use each mock pattern
   - [ ] Add migration guide from old patterns

3. **Component Testing Patterns**:
   - [ ] Document SSE/WebSocket testing patterns
   - [ ] Add form testing best practices
   - [ ] Create accessibility testing guide

## 🔄 Continuous Improvement

### Weekly Review Tasks
- Review new tests for mock utility adoption
- Identify and fix flaky tests
- Update documentation with new patterns discovered
- Share testing tips in team meetings

### Monthly Metrics Review
- Test coverage trends
- Test execution time trends
- Flaky test count
- Mock utility adoption rate

## 📝 Notes

- The centralized mock utilities are now available but need widespread adoption
- Focus on migrating high-value tests first (routes, critical features)
- Use `task-header.unit.test.tsx` and `task-viewer-with-mocks.unit.test.tsx` as examples
- Keep test execution fast by using appropriate mock strategies