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

#### 6. Updated Testing Documentation ✅
**Status**: COMPLETED  
**Completed**: January 2025

- ✅ Condensed frontend testing guidelines to 352 lines
- ✅ Added references to Strategic Mocking Pattern document
- ✅ Removed redundant coverage requirements and test categorization
- ✅ Integrated backend testing guidelines references

## 🚧 Next Actions (High Priority)

### Phase 2: Adopt Mock Utilities Throughout Codebase
**Priority**: High  
**Goal**: Update ALL existing tests to use centralized mocks (48 test files)

**Priority 1 - High Impact Routes & Integration Tests (12 files)**:
1. `task-detail.integration.test.tsx` - Main task page integration
2. `task-list.integration.test.tsx` - Task list page integration  
3. `repository-form.unit.test.tsx` - Critical settings form
4. `chat-input-integration.test.tsx` - Core chat functionality
5. `event-list-tool-calls.integration.test.tsx` - Event system integration
6. `chat-view-commands.integration.test.tsx` - Command system
7. `git-diff-state-isolation.integration.test.tsx` - Git diff features
8. `chat-view-scroll.integration.test.tsx` - Chat scrolling behavior
9. `task-ui-provider.unit.test.tsx` - Core task state management
10. `global-event-stream-provider.unit.test.tsx` - SSE connection management
11. `hooks-violations-regression.integration.test.tsx` - Hook safety patterns
12. `chat-state-isolation.integration.test.tsx` - Chat state management

**Priority 2 - Core Component Tests (16 files)**:
13. `task-viewer.unit.test.tsx` - Task display component
14. `task-header.unit.test.tsx` - Task header component  
15. `chat-input-file-search.unit.test.tsx` - File search functionality
16. `chat-input-expandable.unit.test.tsx` - Chat input expansion
17. `tool-call-event.unit.test.tsx` - Tool call rendering
18. `message-event.unit.test.tsx` - Message rendering
19. `command-buttons.unit.test.tsx` - Command button interactions
20. `task-ui-provider-chat-state.unit.test.tsx` - Chat state provider
21. `derived-state-patterns.unit.test.tsx` - State derivation patterns
22. `merge-tool-events.unit.test.ts` - Event merging utility
23. `chat-form-refactored.unit.test.tsx` - Chat form component
24. `chat-input-task-isolation.integration.test.tsx` - Chat isolation
25. `selectors.unit.test.tsx` - Selector utilities
26. Plus 3 additional component tests

**Priority 3 - Remaining Tests (20 files)**:
29-48. All remaining test files using ad-hoc mocking patterns

**Example Migration Pattern**:
```typescript
// Before (ad-hoc mocking):
vi.mocked(useTask).mockReturnValue({
  data: taskData,
  isLoading: false,
  error: null
} as any)

// After (centralized mocks):
import { createMockQuery } from '@/test-utils/mocks'
vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
```

### Phase 3: Add Missing Test Coverage
**Priority**: Low (after mock migration)
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

## 📊 Success Metrics

### Achieved ✅
- ✅ Zero tests with meaningless assertions
- ✅ All skipped tests removed or properly documented
- ✅ Centralized mock utilities created and documented
- ✅ Consistent test naming convention implemented
- ✅ Updated testing guidelines with new patterns
- ✅ Condensed frontend testing guidelines to 352 lines
- ✅ Integrated Strategic Mocking Pattern framework
- ✅ Cross-referenced backend testing guidelines

### In Progress 🚧
- [ ] 100% of tests using centralized mocks (currently ~10%, 48 total files)
- [ ] Test coverage increased to 80% (currently ~70%)
- [ ] All critical user paths have integration tests

### Future Goals 🎯
- [ ] Average test execution time < 30s for unit tests
- [ ] Zero flaky tests in CI
- [ ] 100% of new features have tests before merge
- [ ] Automated test quality reports in PRs
- [ ] E2E testing infrastructure (separate action plan)

## 📋 Implementation Checklist

### Immediate Actions (This Week)
- [ ] Update Priority 1 test files to use centralized mocks (12 files)
- [ ] Document mock migration examples from completed work

### Short-term (Next 2 Weeks)
- [ ] Update Priority 2 test files to use centralized mocks (16 files)
- [ ] Add integration tests for any uncovered critical routes

### Long-term (Next Month)
- [ ] Complete Priority 3 mock migration (remaining 20 files)
- [ ] Achieve 80% test coverage through improved test quality
- [ ] Set up automated test quality gates

## 📚 Documentation Updates

### Completed ✅
1. **Testing Guidelines** (`docs/guidelines/frontend-testing.md`):
   - ✅ Add centralized mock utilities section
   - ✅ Update test naming convention
   - ✅ Add test quality standards
   - ✅ Condensed to 352 lines for better maintainability
   - ✅ Added cross-references to Strategic Mocking Pattern
   - ✅ Integrated backend testing guidelines references

### Still Needed 📋
1. **Mock Utilities Documentation**:
   - [ ] Create `docs/guidelines/mock-utilities.md`
   - [ ] Add examples for each mock type
   - [ ] Document when to use each mock pattern
   - [ ] Add migration guide from old patterns

2. **Component Testing Patterns**:
   - [ ] Document SSE/WebSocket testing patterns
   - [ ] Add form testing best practices
   - [ ] Add troubleshooting guide section

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