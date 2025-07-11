import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

// Mock TanStack Router
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: vi.fn((path: string) => ({
    useParams: vi.fn(() => ({ taskId: 'test-task-123' })),
  })),
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
    const TaskDetail = Route.component

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
    const TaskDetail = Route.component

    render(<TaskDetail />)

    // Verify TaskProviders was called with correct props
    expect(TaskProviders).toHaveBeenCalledWith(
      expect.objectContaining({
        taskId: 'test-task-123',
      }),
      expect.any(Object)
    )
  })

  it('should use route params hook to get taskId', () => {
    const mockUseParams = vi.fn(() => ({ taskId: 'different-task-456' }))
    Route.useParams = mockUseParams

    const TaskDetail = Route.component
    render(<TaskDetail />)

    expect(mockUseParams).toHaveBeenCalled()
    
    const providers = screen.getByTestId('task-providers')
    expect(providers).toHaveAttribute('data-task-id', 'different-task-456')
  })
})