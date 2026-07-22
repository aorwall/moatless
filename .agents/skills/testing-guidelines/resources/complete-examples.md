# Complete Testing Examples

## Frontend Component Test

```tsx
// src/features/tasks/components/__tests__/task-list.test.tsx
import { describe, test, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { TaskList } from '../task-list'
import { useTasksQuery } from '../../api/use-tasks-query'

vi.mock('../../api/use-tasks-query')

function renderWithProviders(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } }
  })
  return render(
    <QueryClientProvider client={queryClient}>
      {ui}
    </QueryClientProvider>
  )
}

describe('TaskList', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  test('displays loading state', () => {
    useTasksQuery.mockReturnValue({
      data: undefined,
      isLoading: true,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByRole('status')).toBeInTheDocument()
    expect(screen.getByText('Loading tasks...')).toBeInTheDocument()
  })

  test('displays error state', () => {
    useTasksQuery.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error('Failed to load tasks')
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText(/Failed to load tasks/)).toBeInTheDocument()
  })

  test('displays empty state when no tasks', () => {
    useTasksQuery.mockReturnValue({
      data: [],
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText('No tasks yet')).toBeInTheDocument()
  })

  test('displays list of tasks', () => {
    const tasks = [
      { id: '1', title: 'Task 1', status: 'pending' },
      { id: '2', title: 'Task 2', status: 'running' },
      { id: '3', title: 'Task 3', status: 'completed' }
    ]

    useTasksQuery.mockReturnValue({
      data: tasks,
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    expect(screen.getByText('Task 1')).toBeInTheDocument()
    expect(screen.getByText('Task 2')).toBeInTheDocument()
    expect(screen.getByText('Task 3')).toBeInTheDocument()
  })

  test('navigates to task detail on click', async () => {
    const user = userEvent.setup()
    const mockNavigate = vi.fn()

    vi.mock('@tanstack/react-router', () => ({
      useNavigate: () => mockNavigate,
      Link: ({ to, params, children }) => (
        <a href="#" onClick={() => mockNavigate({ to, params })}>
          {children}
        </a>
      )
    }))

    const tasks = [{ id: '1', title: 'Task 1', status: 'pending' }]
    useTasksQuery.mockReturnValue({
      data: tasks,
      isLoading: false,
      error: null
    })

    renderWithProviders(<TaskList />)

    await user.click(screen.getByText('Task 1'))

    expect(mockNavigate).toHaveBeenCalledWith({
      to: '/tasks/$taskId',
      params: { taskId: '1' }
    })
  })
})
```

## Backend Tests

The backend is Rust — its tests run with `cargo test -p moatless-vibe-backend` and follow the conventions in `.claude/rules/`. This skill covers Vitest (frontend and TS packages) only.

## Related Resources
- [Frontend Testing](../../frontend-dev-guidelines/resources/testing.md) - Frontend patterns
- [Testing Philosophy](./testing-philosophy.md) - Core principles
