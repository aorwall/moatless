# Frontend Testing Improvement Action Plan

## Overview
This action plan addresses the testing inconsistencies and gaps discovered during the comprehensive frontend test analysis. The plan is organized by priority and includes specific files to be modified or created.

## Immediate Actions (Week 1)

### 1. Fix Disabled Integration Tests
**Priority**: Critical  
**Goal**: Re-enable critical integration tests that are currently disabled

**Affected Files**:
- `apps/frontend/src/routes/tasks/__tests__/task-detail.integration.test.tsx` ✅ **COMPLETED**

**Tasks**:
- [x] Fix context migration issues preventing these tests from running
- [x] Update tests to use proper TaskProvider setup
- [x] Remove placeholder `expect(true).toBe(true)` assertions
- [x] Add proper integration test coverage for task detail views

**Completed Actions**:
- Created comprehensive integration tests covering:
  - Initial rendering and loading states
  - Chat functionality and message sending
  - Real-time SSE updates
  - Git diff display
  - Workspace and server status
  - Error handling and recovery
  - User interactions (iteration switching, panel toggling)

**Note**: The `task-detail-simple.integration.test.tsx` file can be removed as it's redundant with the comprehensive integration test now in place.

### 2. Remove Meaningless Tests
**Priority**: High  
**Goal**: Clean up tests that provide no value

**Affected Files**:
- All test files containing `expect(true).toBe(true)`
- Tests with permanent `FIXME` comments
- Tests that are permanently skipped without clear documentation

**Tasks**:
- [ ] Audit all test files for placeholder tests
- [ ] Either implement proper tests or remove them entirely
- [ ] Document why any remaining skipped tests are skipped with ticket numbers

### 3. Standardize Mock Utilities
**Priority**: High  
**Goal**: Create centralized mock utilities for consistency

**New Files to Create**:
- `apps/frontend/src/test-utils/mocks/providers.tsx` - Mock provider wrappers
- `apps/frontend/src/test-utils/mocks/api-hooks.ts` - Standardized API hook mocks
- `apps/frontend/src/test-utils/mocks/sse-connections.ts` - SSE/WebSocket mocks
- `apps/frontend/src/test-utils/mocks/ui-libraries.ts` - Complex UI component mocks

**Tasks**:
- [ ] Extract common mock patterns from existing tests
- [ ] Create reusable mock utilities
- [ ] Update existing tests to use centralized mocks
- [ ] Document when to use each mock pattern

## Short-term Improvements (Weeks 2-3)

### 4. Implement Test Naming Convention
**Priority**: Medium  
**Goal**: Adopt clear test type naming conventions

**Affected Files**: All test files (~48 files)

**Naming Convention**:
- `*.unit.test.ts(x)` - Unit tests (fast, isolated)
- `*.integration.test.tsx` - Integration tests (moderate speed)
- `*.e2e.test.ts(x)` - End-to-end tests (slow)
- `*.perf.test.ts(x)` - Performance regression tests

**Tasks**:
- [ ] Rename existing test files to follow convention
- [ ] Update test commands to filter by test type
- [ ] Update CI configuration to run tests by type

### 5. Add Missing Test Types
**Priority**: Medium  
**Goal**: Implement E2E and contract tests

**New Files to Create**:
- `apps/frontend/src/__tests__/e2e/task-creation-flow.e2e.test.tsx`
- `apps/frontend/src/__tests__/e2e/task-real-time-updates.e2e.test.tsx`
- `apps/frontend/src/__tests__/contracts/component-interfaces.test.tsx`
- `apps/frontend/src/__tests__/contracts/hook-signatures.test.tsx`

**Tasks**:
- [ ] Set up E2E testing infrastructure (Playwright/Cypress)
- [ ] Implement critical user journey E2E tests
- [ ] Create contract tests for component interfaces
- [ ] Add performance benchmarks based on existing patterns

### 6. Enhance Test Quality
**Priority**: Medium  
**Goal**: Improve existing tests to test behavior, not implementation

**High-Priority Files to Update**:
- `apps/frontend/src/features/tasks/components/__tests__/task-list.test.tsx`
- `apps/frontend/src/features/tasks/components/__tests__/task-viewer.test.tsx`
- `apps/frontend/src/features/environments/components/__tests__/environment-list.test.tsx`
- `apps/frontend/src/features/preview/components/__tests__/preview-panel.test.tsx`

**Tasks**:
- [ ] Add behavior testing to components that only test rendering
- [ ] Implement comprehensive error state testing
- [ ] Add interaction testing where missing
- [ ] Remove tests that only check for element presence

## Long-term Improvements (Month 2+)

### 7. Update Testing Guidelines
**Priority**: Medium  
**Goal**: Document all discovered patterns and best practices

**Files to Update**:
- `docs/guidelines/frontend-testing.md` - Main testing guide

**New Sections to Add**:
- Performance Testing Patterns
- React Context Testing
- SSE/WebSocket Testing
- Form Testing Strategies
- Accessibility Testing Patterns

### 8. Create Testing Templates
**Priority**: Low  
**Goal**: Provide templates for common test scenarios

**New Files to Create**:
- `apps/frontend/src/test-utils/templates/component-test.template.tsx`
- `apps/frontend/src/test-utils/templates/hook-test.template.ts`
- `apps/frontend/src/test-utils/templates/integration-test.template.tsx`
- `apps/frontend/src/test-utils/templates/context-test.template.tsx`

### 9. Implement Test Quality Gates
**Priority**: Low  
**Goal**: Enforce test standards automatically

**Files to Create/Update**:
- `.github/workflows/test-quality.yml` - CI workflow for test quality
- `apps/frontend/vitest.config.ts` - Coverage thresholds
- `.eslintrc.js` - Add test-specific linting rules

**Tasks**:
- [ ] Set up coverage thresholds per test type
- [ ] Add linting rules for test quality
- [ ] Create pre-commit hooks for test standards
- [ ] Set up test quality dashboard

## File-by-File Action Items

### Critical Files Needing Immediate Attention

1. **`task-detail.integration.test.tsx`**
   - Remove all skipped tests
   - Implement proper TaskProvider setup
   - Add real integration tests

2. **`task-detail-simple.integration.test.tsx`**
   - Fix context migration issues
   - Enable all tests
   - Add missing test cases

3. **`repository-form.test.tsx`**
   - Implement dropdown testing strategy
   - Reduce Zod mocking
   - Add form validation tests

### Files Demonstrating Best Practices (Use as Templates)

1. **`task-list.test.tsx`**
   - Good example of comprehensive component testing
   - Use as template for other list components

2. **`use-preview.test.tsx`**
   - Excellent hook testing pattern
   - Use as template for other hook tests

3. **`preview-reducer.test.tsx`**
   - Great React Context testing example
   - Document this pattern in guidelines

4. **`merge-tool-events.test.ts`**
   - Comprehensive utility testing
   - Use as template for complex utility tests

5. **`infinite-loop-detection.test.tsx`**
   - Innovative performance testing
   - Formalize this pattern for other components

## Success Metrics

### Immediate (Week 1)
- [ ] All integration tests re-enabled and passing
- [ ] Zero tests with `expect(true).toBe(true)`
- [ ] Centralized mock utilities created

### Short-term (Month 1)
- [ ] 100% of tests follow naming convention
- [ ] At least 5 E2E tests implemented
- [ ] Test coverage increased by 10%

### Long-term (Month 2+)
- [ ] Testing guidelines fully updated
- [ ] All critical paths have E2E coverage
- [ ] Test quality gates enforced in CI
- [ ] Average test execution time reduced by 20%

## Implementation Order

1. **Week 1**: Fix broken tests and clean up
2. **Week 2**: Implement naming conventions and mock utilities
3. **Week 3**: Add E2E infrastructure and first tests
4. **Week 4**: Update guidelines and create templates
5. **Month 2**: Implement quality gates and monitoring

## Notes

- Prioritize fixing disabled integration tests as they test critical user flows
- Use existing high-quality tests as templates when improving others
- Focus on testing behavior over implementation details
- Ensure all changes are backward compatible with existing test commands