---
name: testing-guidelines
description: Vitest testing patterns with tier classification, strategic mocking, unit tests, integration tests, component testing, React Testing Library, test structure, mocking strategies, test naming conventions, coverage goals, test organization, frontend testing, backend testing, E2E testing patterns
---

# Testing Guidelines

## Purpose
This skill provides comprehensive testing guidance for the Moatless Vibe application using Vitest, React Testing Library, and strategic mocking approaches.

## Technology Stack

### Testing Tools
- **Test Runner**: Vitest
- **Component Testing**: React Testing Library
- **User Interaction**: @testing-library/user-event
- **Mocking**: Vitest mocking utilities
- **Coverage**: Vitest coverage (c8)

## Quick Reference

| Need to... | See |
|------------|-----|
| Understand testing philosophy | [Testing Philosophy](resources/testing-philosophy.md) |
| Classify test importance | [Tier Classification](resources/tier-classification.md) |
| Mock dependencies | [Strategic Mocking](resources/strategic-mocking.md) |
| Write component tests | Frontend skill: [Testing](../frontend-dev-guidelines/resources/testing.md) |
| Write service tests | Backend skill: [Testing](../backend-dev-guidelines/resources/testing.md) |
| See test examples | [Complete Examples](resources/complete-examples.md) |

## Core Principles

### 1. Test Behavior, Not Implementation
Focus on what the code does, not how it does it:

**❌ Wrong**:
```tsx
test('has correct internal state', () => {
  const { result } = renderHook(() => useTask())
  expect(result.current.internalState).toBe('foo')  // Implementation detail
})
```

**✅ Correct**:
```tsx
test('displays loading state', () => {
  render(<TaskCard />)
  expect(screen.getByText('Loading...')).toBeInTheDocument()  // User-visible behavior
})
```

### 2. Strategic Mocking
- Mock external dependencies (APIs, browser APIs, third-party libraries)
- Don't mock your own application code
- Test real component integration

### 3. Tier-Based Testing
Prioritize testing efforts:
- **Tier 1 (80-90% coverage)**: Complex state machines, critical business logic
- **Tier 2 (50-60% coverage)**: Conditional rendering, data fetching
- **Tier 3 (0% coverage)**: Simple presentational components

### 4. Fast Tests
- Unit tests: <200ms
- Integration tests: 200ms-2s
- E2E tests: >2s

### 5. Clear Test Names
Use descriptive test names that explain what's being tested:

**❌ Wrong**:
```tsx
test('works', () => { ... })
test('test1', () => { ... })
```

**✅ Correct**:
```tsx
test('creates task with valid title', () => { ... })
test('displays error when title is too short', () => { ... })
```

## Testing Philosophy

### What to Test
✅ **Test these**:
- User interactions and workflows
- Data transformations and business logic
- Error states and edge cases
- Loading states and async behavior
- Integration between your components
- API integration points

### What NOT to Test
❌ **Skip these**:
- Implementation details (internal state, private methods)
- Third-party library functionality
- Simple presentational components
- CSS classes and styling
- Trivial getters/setters

## Test Structure

### Arrange-Act-Assert Pattern
```tsx
test('creates task successfully', async () => {
  // Arrange - Setup
  const user = userEvent.setup()
  render(<CreateTaskForm />)

  // Act - User action
  await user.type(screen.getByLabelText('Title'), 'New Task')
  await user.click(screen.getByRole('button', { name: 'Create' }))

  // Assert - Verify result
  expect(await screen.findByText('Task created!')).toBeInTheDocument()
})
```

### Test Organization
```
src/features/tasks/
├── components/
│   ├── task-card.tsx
│   └── __tests__/
│       └── task-card.test.tsx
├── services/
│   ├── task.service.ts
│   └── __tests__/
│       ├── task.service.test.ts        # Unit
│       └── task.service.integration.test.ts  # Integration
└── api/
    ├── use-tasks-query.ts
    └── __tests__/
        └── use-tasks-query.test.ts
```

## Test Naming Conventions

### File Naming
```bash
# Unit tests (fast, <200ms)
component-name.test.tsx
service-name.test.ts
hook-name.test.ts

# Integration tests (moderate, 200ms-2s)
feature-name.integration.test.ts
api-integration.integration.test.ts

# E2E tests (slow, >2s)
workflow-name.e2e.test.ts
```

### Test Descriptions
```tsx
describe('TaskService', () => {
  describe('createTask', () => {
    test('creates task with valid data', () => { ... })
    test('throws error when title is too short', () => { ... })
    test('sets default status to pending', () => { ... })
  })

  describe('updateTask', () => {
    test('updates task successfully', () => { ... })
    test('throws error when task not found', () => { ... })
  })
})
```

## Running Tests

### Commands
```bash
# All tests (excludes E2E)
bun run test

# Unit tests only (fastest)
bun run test:unit

# Integration tests only
bun run test:integration

# E2E tests only (slowest)
bun run test:e2e

# With coverage
bun run test:coverage

# Watch mode
bun run test --watch

# Specific file
bun run test task.service.test.ts
```

### Coverage Goals
- **Overall**: 70-80%
- **Critical paths** (Tier 1): 80-90%
- **Conditional logic** (Tier 2): 50-60%
- **Presentational** (Tier 3): 0%

## Common Patterns

### Frontend Component Test
```tsx
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { TaskCard } from './task-card'

test('displays task title and status', () => {
  const task = { id: '1', title: 'Test Task', status: 'pending' }
  render(<TaskCard task={task} />)

  expect(screen.getByText('Test Task')).toBeInTheDocument()
  expect(screen.getByText('pending')).toBeInTheDocument()
})
```

### Backend Service Test
```tsx
import { describe, test, expect, vi, beforeEach } from 'vitest'
import { TaskService } from '../task.service'

describe('TaskService', () => {
  let taskService: TaskService
  let mockTaskRepo: ITaskRepository

  beforeEach(() => {
    mockTaskRepo = {
      findAll: vi.fn(),
      create: vi.fn()
    }
    taskService = new TaskService(mockTaskRepo)
  })

  test('getAllTasks returns all tasks', async () => {
    mockTaskRepo.findAll.mockResolvedValue([{ id: '1', title: 'Task' }])

    const tasks = await taskService.getAllTasks()

    expect(tasks).toHaveLength(1)
    expect(mockTaskRepo.findAll).toHaveBeenCalled()
  })
})
```

## Anti-Patterns

❌ **Testing implementation details**
❌ **Mocking your own code**
❌ **Testing third-party libraries**
❌ **Overly specific assertions**
❌ **Not testing error states**
❌ **Skipping async behavior tests**

See pattern documentation for detailed anti-patterns.

## Navigation Guide

### Essential Resources
1. **[Testing Philosophy](resources/testing-philosophy.md)** - Core testing principles
2. **[Tier Classification](resources/tier-classification.md)** - Prioritize testing efforts
3. **[Strategic Mocking](resources/strategic-mocking.md)** - What to mock, what not to

### Specific Testing Guides
4. **Frontend Testing**: See [Frontend Testing](../frontend-dev-guidelines/resources/testing.md)
5. **Backend Testing**: See [Backend Testing](../backend-dev-guidelines/resources/testing.md)

### Examples
6. **[Complete Examples](resources/complete-examples.md)** - Full test suites

## Quick Start Checklist

### Before Writing Tests
- [ ] Identify test tier (1, 2, or 3)
- [ ] Determine what to mock (external only)
- [ ] Plan test cases (success, errors, edge cases)
- [ ] Choose appropriate test type (unit vs integration)

### Writing the Test
- [ ] Use descriptive test names
- [ ] Follow Arrange-Act-Assert pattern
- [ ] Test behavior, not implementation
- [ ] Mock external dependencies only
- [ ] Assert on user-visible behavior

### After Writing Tests
- [ ] Tests pass consistently
- [ ] Coverage meets tier goals
- [ ] Tests run quickly (<200ms for unit tests)
- [ ] No flaky tests

## Getting Help

### When Testing...
- **Components**: See [Frontend Testing](../frontend-dev-guidelines/resources/testing.md)
- **Services**: See [Backend Testing](../backend-dev-guidelines/resources/testing.md)
- **Hooks**: See [Frontend Testing](../frontend-dev-guidelines/resources/testing.md)
- **API Integration**: See strategic mocking guides
