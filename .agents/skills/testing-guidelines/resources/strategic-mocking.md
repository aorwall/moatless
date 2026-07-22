# Strategic Mocking

## Core Principle
**Mock external dependencies, test real component integration.**

## What to Mock ✅

### External Dependencies
```tsx
// ✅ Mock API calls
vi.mock('@/features/tasks/api/use-tasks-query', () => ({
  useTasksQuery: vi.fn()
}))

// ✅ Mock browser APIs
vi.mock('window.localStorage')

// ✅ Mock navigation
const mockNavigate = vi.fn()
vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => mockNavigate
}))

// ✅ Mock date/time
vi.useFakeTimers()
vi.setSystemTime(new Date('2025-01-01'))
```

## What NOT to Mock ❌

### Your Application Code
```tsx
// ❌ DON'T mock your own components
vi.mock('./task-card', () => ({
  TaskCard: () => <div>Mocked TaskCard</div>
}))

// ❌ DON'T mock your own hooks (unless they call external APIs)
vi.mock('./use-task-form')

// ❌ DON'T mock your own utilities
vi.mock('./format-date')
```

### Test Real Integration
```tsx
// ✅ GOOD - Test real component integration
test('displays list of tasks', () => {
  useTasksQuery.mockReturnValue({
    data: [
      { id: '1', title: 'Task 1' },
      { id: '2', title: 'Task 2' }
    ],
    isLoading: false,
    error: null
  })

  render(<TaskList />)  // Real TaskList with real TaskCard children

  expect(screen.getByText('Task 1')).toBeInTheDocument()
  expect(screen.getByText('Task 2')).toBeInTheDocument()
})
```

## Mocking Patterns

### Mock API Query Hooks
```tsx
import { vi } from 'vitest'
import { useTasksQuery } from '@/features/tasks/api/use-tasks-query'

vi.mock('@/features/tasks/api/use-tasks-query', () => ({
  useTasksQuery: vi.fn()
}))

test('displays tasks', () => {
  useTasksQuery.mockReturnValue({
    data: [{ id: '1', title: 'Task' }],
    isLoading: false,
    error: null
  })

  render(<TaskList />)
  expect(screen.getByText('Task')).toBeInTheDocument()
})

test('displays loading state', () => {
  useTasksQuery.mockReturnValue({
    data: undefined,
    isLoading: true,
    error: null
  })

  render(<TaskList />)
  expect(screen.getByText('Loading...')).toBeInTheDocument()
})
```

### Mock Mutation Hooks
```tsx
vi.mock('@/features/tasks/api/use-create-task-mutation', () => ({
  useCreateTaskMutation: vi.fn()
}))

test('creates task on form submit', async () => {
  const mockMutate = vi.fn()
  useCreateTaskMutation.mockReturnValue({
    mutate: mockMutate,
    isPending: false
  })

  const user = userEvent.setup()
  render(<CreateTaskForm />)

  await user.type(screen.getByLabelText('Title'), 'New Task')
  await user.click(screen.getByRole('button', { name: 'Create' }))

  expect(mockMutate).toHaveBeenCalledWith({ title: 'New Task' })
})
```

## When to Mock Your Own Code

### Exception: External API Wrappers
If your code directly wraps an external service, mocking the wrapper is mocking that external boundary:

```tsx
// ✅ OK to mock this wrapper
export function useAuth() {
  return useSupabaseAuth()  // External service
}

vi.mock('@/lib/hooks/use-auth')
```

## Anti-Patterns

❌ **Over-mocking**
```tsx
// WRONG - Mocking everything
vi.mock('./task-card')
vi.mock('./task-list')
vi.mock('./task-header')
vi.mock('./use-task-form')
vi.mock('./format-date')

test('app renders', () => {
  render(<App />)  // Testing nothing!
})
```

❌ **Mocking to Pass Tests**
```tsx
// WRONG - Mocking because test is hard to write
vi.mock('./complex-component', () => ({
  ComplexComponent: () => <div>Mocked</div>
}))

// CORRECT - Fix the component to be testable
// Make it accept props, separate concerns, etc.
```
