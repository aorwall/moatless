# Test Improvement Action Plan

## Overview

This action plan outlines specific tasks to improve test maintainability and reduce brittleness in the Moatless Swarm UI test suite. Tasks are prioritized by impact and effort.

## Phase 1: Quick Wins (1-2 days)

### 1.1 Add Data-TestId Attributes
**Priority**: HIGH | **Effort**: LOW

Add stable test identifiers to frequently tested components:

```typescript
// Components to update:
- MessageInput: data-testid="message-input", "submit-button"
- GenericSelector: data-testid="model-selector", "repo-selector", "branch-selector"
- TaskList: data-testid="task-row-{taskId}", "delete-button-{taskId}"
- RepositoryForm: data-testid="name-input", "path-input", "submit-button"
- ChatInput: data-testid="chat-textarea", "send-button", "model-selector"
```

**Files to modify:**
- `/apps/frontend/src/features/tasks/components/create-task/message-input.tsx`
- `/apps/frontend/src/features/tasks/components/chat-input/selectors/generic-selector.tsx`
- `/apps/frontend/src/features/tasks/components/task-list.tsx`
- `/apps/frontend/src/features/settings/components/repository-form.tsx`
- `/apps/frontend/src/features/tasks/components/chat-input/index.tsx`

### 1.2 Fix Repository Form Validation Tests
**Priority**: HIGH | **Effort**: MEDIUM

Current issue: ZodError throws uncaught exceptions in tests.

**Solution**: Test validation logic separately from UI
```typescript
// Create new test file: repository-validation.test.ts
describe('Repository Form Validation', () => {
  it('validates required fields', () => {
    const result = CreateRepositorySchema.safeParse({})
    expect(result.success).toBe(false)
    expect(result.error.issues).toContainEqual(
      expect.objectContaining({ path: ['name'] })
    )
  })
})
```

**Files to create:**
- `/apps/frontend/src/features/settings/utils/__tests__/repository-validation.test.ts`

**Files to modify:**
- Remove validation tests from `repository-form.test.tsx`
- Update form component to expose validation state for testing

### 1.3 Create Centralized Mock Utilities
**Priority**: HIGH | **Effort**: LOW

Create reusable mocks to reduce duplication:

**Files to create:**
- `/apps/frontend/src/test-utils/mocks/selectors.tsx`
- `/apps/frontend/src/test-utils/mocks/file-search.ts`
- `/apps/frontend/src/test-utils/mocks/api-hooks.ts`

```typescript
// test-utils/mocks/selectors.tsx
export const mockGenericSelector = () => ({
  GenericSelector: ({ value, items, testId }: any) => (
    <div data-testid={testId}>
      {items?.find(i => i.id === value)?.label}
    </div>
  ),
  useRepositorySelector: vi.fn(),
  useModelSelector: vi.fn(),
  useBranchSelector: vi.fn(),
  useTemplateSelector: vi.fn(),
})
```

## Phase 2: Component Refactoring (3-5 days)

### 2.1 Extract Business Logic from Components
**Priority**: HIGH | **Effort**: MEDIUM

Extract payload creation and validation logic:

**CreateCodeTask refactoring:**
```typescript
// New file: create-task-utils.ts
export const createCodeTaskPayload = (
  message: string,
  repositoryId: string,
  model: string,
  branch: string
): CreateTaskRequest => ({
  agentType: 'claude-code',
  message: message.trim(),
  repositoryId,
  model,
  filePaths: [],
  originalBranch: branch,
})

// Test separately
it('creates valid code task payload', () => {
  const payload = createCodeTaskPayload('test', 'repo', 'model', 'main')
  expect(payload).toMatchObject({
    agentType: 'claude-code',
    filePaths: [],
  })
})
```

**Files to create:**
- `/apps/frontend/src/features/tasks/utils/create-task-utils.ts`
- `/apps/frontend/src/features/tasks/utils/__tests__/create-task-utils.test.ts`

**Files to refactor:**
- `/apps/frontend/src/features/tasks/components/create-task/create-code-task.tsx`
- `/apps/frontend/src/features/tasks/components/create-task/create-vibe-task.tsx`

### 2.2 Implement Component Test Helpers
**Priority**: MEDIUM | **Effort**: MEDIUM

Create test helper classes for complex components:

**Files to create:**
- `/apps/frontend/src/test-utils/component-testers/chat-input.tester.ts`
- `/apps/frontend/src/test-utils/component-testers/task-list.tester.ts`
- `/apps/frontend/src/test-utils/component-testers/repository-form.tester.ts`

### 2.3 Remove Over-Mocked Tests
**Priority**: MEDIUM | **Effort**: LOW

Identify and fix tests that mock too much:

**Tests to review and fix:**
- Any test mocking `TaskViewer`, `EventViewer`, or other business components
- Tests that only verify mock calls instead of behavior
- Tests with placeholder assertions like `expect(true).toBe(true)`

## Phase 3: Test Infrastructure (1 week)

### 3.1 Create Factory Enhancement
**Priority**: MEDIUM | **Effort**: MEDIUM

Enhance existing factories with builder pattern:

```typescript
// test-utils/factories/task.factory.ts
export class TaskBuilder {
  private task: Partial<Task> = { /* defaults */ }
  
  withStatus(status: Task['status']) {
    this.task.status = status
    return this
  }
  
  withIterations(count: number) {
    this.task.iterations = createMockIterations(count)
    return this
  }
  
  build(): Task {
    return createMockTask(this.task)
  }
}
```

### 3.2 Implement Visual Regression Tests
**Priority**: LOW | **Effort**: HIGH

For critical UI components, add visual regression tests:
- Task status badges
- Git sync status indicators
- Form validation states

### 3.3 Document Testing Patterns
**Priority**: MEDIUM | **Effort**: LOW

Create example test files showing best practices:
- `/apps/frontend/src/test-examples/component-unit.example.tsx`
- `/apps/frontend/src/test-examples/route-integration.example.tsx`
- `/apps/frontend/src/test-examples/hook-testing.example.ts`

## Phase 4: Cleanup and Maintenance (Ongoing)

### 4.1 Remove Unnecessary Tests
**Priority**: LOW | **Effort**: LOW

Tests to remove:
- Tests that verify prop passing without behavior
- Tests for CSS classes
- Tests that duplicate framework testing

### 4.2 Update Test Documentation
**Priority**: MEDIUM | **Effort**: LOW

- Update testing guide with new patterns
- Add troubleshooting section
- Create test writing checklist

### 4.3 Establish Test Review Process
**Priority**: HIGH | **Effort**: LOW

- Require data-testid for new interactive elements
- Enforce separation of logic and UI
- Review mock usage in PRs

## Success Metrics

Track improvement through:
1. **Test Stability**: Reduce test failures from UI changes by 80%
2. **Mock Reduction**: Decrease mock setup code by 50%
3. **Test Speed**: Improve test suite runtime by 30%
4. **Maintenance Time**: Reduce time spent fixing tests by 60%

## Implementation Order

1. **Week 1**: Phase 1 (Quick Wins)
2. **Week 2**: Phase 2.1-2.2 (Logic Extraction & Test Helpers)
3. **Week 3**: Phase 2.3 & Phase 3.1 (Remove Over-mocking & Factories)
4. **Ongoing**: Phase 4 (Cleanup & Maintenance)

## Notes

- Prioritize tests that fail frequently
- Update tests incrementally - don't rewrite everything at once
- Focus on high-value components first (task creation, chat input, repository management)
- Consider running a "test improvement sprint" to tackle Phase 1 quickly