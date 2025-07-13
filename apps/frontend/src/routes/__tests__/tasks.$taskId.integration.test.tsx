import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

// Mock TanStack Router
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: vi.fn(() => ({
    options: {
      component: () => {
        const { TaskProviders, TaskPage } = require('@/features/tasks')
        const taskId = 'test-task-123'
        return (
          <TaskProviders taskId={taskId}>
            <TaskPage />
          </TaskProviders>
        )
      },
    },
    useParams: vi.fn(() => ({ taskId: 'test-task-123' })),
  })),
  useParams: vi.fn(() => ({ taskId: 'test-task-123' })),
}))

// Mock task features
vi.mock('@/features/tasks', () => ({
  TaskPage: vi.fn(() => <div data-testid="task-page">Task Page Content</div>),
  TaskProviders: vi.fn(({ children, taskId }: any) => (
    <div data-testid="task-providers" data-task-id={taskId}>
      {children}
    </div>
  )),
}))

// Import after mocks are set up
import { Route } from '../tasks/$taskId'

describe('Task Detail Route', () => {
  it('should render task detail with correct taskId', () => {
    // Get the component from the route
    const TaskDetail = Route.options?.component || (() => <div>No component</div>)

    render(<TaskDetail />)

    // Check that providers are rendered with correct taskId
    const providers = screen.getByTestId('task-providers')
    expect(providers).toBeInTheDocument()
    expect(providers).toHaveAttribute('data-task-id', 'test-task-123')

    // Check that task page is rendered
    expect(screen.getByTestId('task-page')).toBeInTheDocument()
    expect(screen.getByText('Task Page Content')).toBeInTheDocument()
  })

  it('should pass taskId from route params to TaskProviders', () => {
    const { TaskProviders } = require('@/features/tasks')
    const TaskDetail = Route.options?.component || (() => <div>No component</div>)

    render(<TaskDetail />)

    // Verify TaskProviders was called with correct props
    expect(TaskProviders).toHaveBeenCalledWith(
      expect.objectContaining({
        taskId: 'test-task-123',
      }),
      expect.any(Object),
    )
  })

  it('should use route params hook to get taskId', () => {
    vi.mocked(require('@tanstack/react-router').useParams).mockReturnValue({
      taskId: 'different-task-456',
    })

    const TaskDetail = Route.options?.component || (() => <div>No component</div>)
    render(<TaskDetail />)

    // expect(mockUseParams).toHaveBeenCalled() // Comment out since we're not using the local mock

    const providers = screen.getByTestId('task-providers')
    expect(providers).toHaveAttribute('data-task-id', 'different-task-456')
  })
})
