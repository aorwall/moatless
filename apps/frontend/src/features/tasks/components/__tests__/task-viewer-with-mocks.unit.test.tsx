import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useTask } from '@/features/tasks/hooks/use-tasks'
import { useTaskEvents, useTaskStatus } from '@/features/tasks/contexts/task-providers'
import { TaskViewer } from '../task-viewer'
import {
  createMockQuery,
  createMockLoadingQuery,
  createMockErrorQuery,
  createQueryWrapper,
  mockResizableComponents,
} from '@/test-utils/mocks'

// Mock dependencies
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/tasks/contexts/task-providers')

// Mock resizable UI components
vi.mock('@/lib/components/ui/resizable', () => mockResizableComponents())

// Mock router
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task-123' }),
}))

describe('TaskViewer with Mock Utilities', () => {
  const wrapper = createQueryWrapper()

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Loading States', () => {
    it('displays loading skeleton when task is loading', () => {
      vi.mocked(useTask).mockReturnValue(createMockLoadingQuery())
      vi.mocked(useTaskStatus).mockReturnValue({ status: 'loading' } as any)
      vi.mocked(useTaskEvents).mockReturnValue(createMockLoadingQuery())

      render(<TaskViewer />, { wrapper })

      // Should show loading skeleton
      expect(screen.getByTestId('task-viewer-skeleton')).toBeInTheDocument()
    })
  })

  describe('Error States', () => {
    it('displays error message when task fetch fails', () => {
      const error = new Error('Failed to fetch task')
      vi.mocked(useTask).mockReturnValue(createMockErrorQuery(error))
      vi.mocked(useTaskStatus).mockReturnValue({ status: 'error' } as any)
      vi.mocked(useTaskEvents).mockReturnValue(createMockQuery([]))

      render(<TaskViewer />, { wrapper })

      // Should show error alert
      expect(screen.getByRole('alert')).toBeInTheDocument()
      expect(screen.getByText(/Failed to fetch task/)).toBeInTheDocument()
    })
  })

  describe('Success States', () => {
    it('displays task data when loaded successfully', () => {
      const mockTask = {
        id: 'test-task-123',
        title: 'Test Task',
        status: 'running',
        iterations: [
          {
            id: 1,
            message: 'First iteration',
            status: 'completed',
          },
          {
            id: 2,
            message: 'Second iteration',
            status: 'running',
          },
        ],
      }

      const mockEvents = [
        { id: '1', type: 'task:started', data: {} },
        { id: '2', type: 'message', data: { content: 'Hello' } },
      ]

      vi.mocked(useTask).mockReturnValue(createMockQuery(mockTask))
      vi.mocked(useTaskStatus).mockReturnValue({ status: 'running' } as any)
      vi.mocked(useTaskEvents).mockReturnValue(createMockQuery(mockEvents))

      render(<TaskViewer />, { wrapper })

      // Should display task iterations
      expect(screen.getByText('First iteration')).toBeInTheDocument()
      expect(screen.getByText('Second iteration')).toBeInTheDocument()

      // Should display events
      expect(screen.getByText('task:started')).toBeInTheDocument()
      expect(screen.getByText('Hello')).toBeInTheDocument()
    })
  })

  describe('Empty States', () => {
    it('displays empty state when task has no iterations', () => {
      const mockTask = {
        id: 'test-task-123',
        title: 'Empty Task',
        status: 'open',
        iterations: [],
      }

      vi.mocked(useTask).mockReturnValue(createMockQuery(mockTask))
      vi.mocked(useTaskStatus).mockReturnValue({ status: 'open' } as any)
      vi.mocked(useTaskEvents).mockReturnValue(createMockQuery([]))

      render(<TaskViewer />, { wrapper })

      expect(screen.getByText(/No iterations yet/)).toBeInTheDocument()
    })
  })
})