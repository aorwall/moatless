# Frontend Testing

## Table of Contents
- [Overview](#overview)
- [Testing Tier System](#testing-tier-system)
- [Strategic Mocking](#strategic-mocking)
- [Component Testing](#component-testing)
- [Hook Testing](#hook-testing)
- [Integration Testing](#integration-testing)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

This project uses a tier-based testing approach focused on behavior, not implementation.

### Technology Stack
- `vitest` - Test runner
- `@testing-library/react` - Component testing
- `@testing-library/user-event` - User interaction simulation
- Strategic mocking approach

### Key Principles
- Test behavior, not implementation
- Mock external dependencies, test real component integration
- Use tier system to prioritize testing efforts
- Never test private methods or implementation details

## Testing Tier System

### Tier 1: Essential (80-90% Coverage)
**What to test**: Complex state machines, business logic, critical user flows

```tsx
// MUST TEST: Complex state logic
function useTaskStateMachine() {
  const [status, setStatus] = useState<TaskStatus>('idle')

  const start = () => setStatus('running')
  const complete = () => setStatus('completed')
  const fail = () => setStatus('failed')

  return { status, start, complete, fail }
}
```

**Examples**:
- Multi-step forms
- Complex user interactions
- Data transformations
- Critical business logic

### Tier 2: Moderate (50-60% Coverage)
**What to test**: Conditional rendering, data fetching, error handling

```tsx
// SHOULD TEST: Conditional rendering logic
function TaskCard({ task }: { task: Task }) {
  if (!task) return <EmptyState />

  if (task.status === 'running') {
    return <RunningTask task={task} />
  }

  return <CompletedTask task={task} />
}
```

**Examples**:
- Components with conditional rendering
- API integration
- Error states
- Loading states

### Tier 3: Skip (0% Coverage)
**What NOT to test**: Pure presentational components

```tsx
// DON'T TEST: Simple presentational component
function TaskTitle({ title }: { title: string }) {
  return <h1 className="text-2xl font-bold">{title}</h1>
}
```

**Skip**:
- Simple display components
- Static layouts
- CSS classes
- Third-party library components

## Strategic Mocking

### What to Mock
✅ **Mock external dependencies**:
- API calls (HTTP requests)
- Browser APIs (localStorage, fetch)
- Third-party services
- Navigation functions
- Date/time functions

### What NOT to Mock
❌ **Don't mock application code**:
- Your own components
- Your own hooks
- Application state
- Component props

### Mocking Examples

**Mock API Calls**:
```tsx
import { vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { TaskList } from './task-list'

vi.mock('@/features/tasks/api/use-tasks-query', () => ({
  useTasksQuery: vi.fn()
}))

test('displays tasks', () => {
  const mockTasks = [
    { id: '1', title: 'Task 1', status: 'pending' }
  ]

  useTasksQuery.mockReturnValue({
    data: mockTasks,
    isLoading: false,
    error: null
  })

  render(<TaskList />)

  expect(screen.getByText('Task 1')).toBeInTheDocument()
})
```

**Mock Navigation**:
```tsx
import { vi } from 'vitest'

const mockNavigate = vi.fn()
vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => mockNavigate
}))

test('navigates on click', async () => {
  const user = userEvent.setup()

  render(<TaskCard task={mockTask} />)

  await user.click(screen.getByText('View Details'))

  expect(mockNavigate).toHaveBeenCalledWith({
    to: '/tasks/$taskId',
    params: { taskId: '1' }
  })
})
```

## Component Testing

### Basic Component Test
```tsx
import { render, screen } from '@testing-library/react'
import { TaskCard } from './task-card'

describe('TaskCard', () => {
  test('renders task title', () => {
    const task = { id: '1', title: 'Test Task', status: 'pending' }

    render(<TaskCard task={task} />)

    expect(screen.getByText('Test Task')).toBeInTheDocument()
  })

  test('shows status badge', () => {
    const task = { id: '1', title: 'Test Task', status: 'completed' }

    render(<TaskCard task={task} />)

    expect(screen.getByText('completed')).toBeInTheDocument()
  })
})
```

### User Interaction Testing
```tsx
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { TaskForm } from './task-form'

describe('TaskForm', () => {
  test('submits form with entered data', async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn()

    render(<TaskForm onSubmit={onSubmit} />)

    // Type into input
    await user.type(screen.getByLabelText('Task Title'), 'New Task')

    // Click submit button
    await user.click(screen.getByRole('button', { name: 'Submit' }))

    // Assert function called with correct data
    expect(onSubmit).toHaveBeenCalledWith({ title: 'New Task' })
  })
})
```

### Testing with React Query
Use the custom render from `@/test-utils/render` — it wraps the tree in a fresh `QueryClient` and mock providers automatically:

```tsx
import { render, screen } from '@/test-utils/render'

describe('TaskList', () => {
  test('displays loading state', () => {
    useTasksQuery.mockReturnValue({ data: undefined, isLoading: true, error: null })
    render(<TaskList />)
    expect(screen.getByText('Loading...')).toBeInTheDocument()
  })
})
```

## Hook Testing

### Testing Custom Hooks
```tsx
import { renderHook, waitFor } from '@testing-library/react'
import { useTaskStateMachine } from './use-task-state-machine'

describe('useTaskStateMachine', () => {
  test('transitions from idle to running', () => {
    const { result } = renderHook(() => useTaskStateMachine())

    expect(result.current.status).toBe('idle')

    result.current.start()

    expect(result.current.status).toBe('running')
  })

  test('completes task', () => {
    const { result } = renderHook(() => useTaskStateMachine())

    result.current.start()
    result.current.complete()

    expect(result.current.status).toBe('completed')
  })
})
```

### Testing Hooks with React Query
```tsx
import { renderHook, waitFor } from '@testing-library/react'
import { QueryClientProvider } from '@tanstack/react-query'
import { createTestQueryClient } from '@/test-utils/render'
import { useTasksQuery } from './use-tasks-query'

describe('useTasksQuery', () => {
  test('fetches tasks successfully', async () => {
    const queryClient = createTestQueryClient()
    const wrapper = ({ children }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    )

    const { result } = renderHook(() => useTasksQuery(), { wrapper })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    expect(result.current.data).toHaveLength(3)
  })
})
```

## Integration Testing

### Testing Full User Flows
```tsx
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { App } from './app'

describe('Task Creation Flow', () => {
  test('user can create a new task', async () => {
    const user = userEvent.setup()

    render(<App />)

    // Navigate to create task page
    await user.click(screen.getByRole('link', { name: 'New Task' }))

    // Fill out form
    await user.type(screen.getByLabelText('Title'), 'My New Task')
    await user.type(screen.getByLabelText('Description'), 'Task description')

    // Submit form
    await user.click(screen.getByRole('button', { name: 'Create' }))

    // Verify success message
    expect(await screen.findByText('Task created!')).toBeInTheDocument()

    // Verify task appears in list
    expect(await screen.findByText('My New Task')).toBeInTheDocument()
  })
})
```

### Testing Route Navigation
```tsx
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { RouterProvider, createMemoryHistory } from '@tanstack/react-router'

describe('Navigation', () => {
  test('navigates to task detail', async () => {
    const user = userEvent.setup()
    const history = createMemoryHistory({ initialEntries: ['/tasks'] })

    render(<RouterProvider router={router} history={history} />)

    // Click on a task
    await user.click(screen.getByText('Task 1'))

    // Verify URL changed
    expect(history.location.pathname).toBe('/tasks/1')

    // Verify task detail displayed
    expect(screen.getByText('Task Details')).toBeInTheDocument()
  })
})
```

## Common Patterns

### Test Setup Helper
The repo-wide helper lives in `src/test-utils/render.tsx`: it exports `createTestQueryClient()` and a custom `render` (re-exporting everything from `@testing-library/react`) that wraps components in `QueryClientProvider` + mock providers. Use it instead of hand-rolling wrappers.

### Custom Matchers
```tsx
// test-utils/matchers.ts
import { expect } from 'vitest'

expect.extend({
  toHaveTask(received: Task[], taskId: string) {
    const task = received.find(t => t.id === taskId)
    const pass = !!task

    return {
      pass,
      message: () =>
        pass
          ? `Expected tasks not to contain ${taskId}`
          : `Expected tasks to contain ${taskId}`
    }
  }
})

// Usage
expect(tasks).toHaveTask('task-1')
```

### Waiting for Async Updates
```tsx
import { waitFor } from '@testing-library/react'

test('updates task after mutation', async () => {
  const user = userEvent.setup()

  render(<TaskDetail taskId="1" />)

  await user.click(screen.getByRole('button', { name: 'Complete' }))

  await waitFor(() => {
    expect(screen.getByText('completed')).toBeInTheDocument()
  })
})
```

## Anti-Patterns

### ❌ Testing Implementation Details
```tsx
// BAD: Testing internal state
test('has correct state', () => {
  const { result } = renderHook(() => useTask())
  expect(result.current.internalState).toBe('foo')  // DON'T!
})

// GOOD: Test behavior
test('displays loading state', () => {
  render(<TaskCard />)
  expect(screen.getByText('Loading...')).toBeInTheDocument()
})
```

### ❌ Testing CSS Classes
```tsx
// BAD: Testing classes
test('has correct class', () => {
  render(<Button />)
  expect(screen.getByRole('button')).toHaveClass('px-4 py-2')  // DON'T!
})

// GOOD: Test visual result if necessary
test('button is styled correctly', () => {
  render(<Button />)
  const button = screen.getByRole('button')
  expect(button).toHaveStyle({ padding: '16px' })
})
```

### ❌ Mocking Your Own Code
```tsx
// BAD: Mocking application component
vi.mock('./task-card', () => ({
  TaskCard: () => <div>Mocked TaskCard</div>
}))

// GOOD: Test real component integration
test('renders task card', () => {
  render(<TaskList />)
  // Test actual TaskCard rendering
})
```

### ❌ Testing Third-Party Libraries
```tsx
// BAD: Testing React Query
test('useQuery returns data', () => {
  const { result } = renderHook(() => useQuery(...))
  expect(result.current.data).toBeDefined()  // Testing library, not your code!
})

// GOOD: Test your query hook usage
test('displays tasks from query', async () => {
  render(<TaskList />)
  expect(await screen.findByText('Task 1')).toBeInTheDocument()
})
```

### ❌ Overly Specific Assertions
```tsx
// BAD: Too specific, breaks easily
expect(container.innerHTML).toBe('<div class="text-lg">Task 1</div>')

// GOOD: Test what user sees
expect(screen.getByText('Task 1')).toBeInTheDocument()
```
