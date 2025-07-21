import type { Task } from '@moatless/types'
import { createMockTask } from '@moatless/types/test-utils'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useDeleteTask } from '@/features/tasks/hooks/use-tasks'
import { createMockMutation, createMockQuery } from '@/test-utils/mocks'
import { TaskList } from '../task-list'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    to,
    params,
  }: {
    children: React.ReactNode
    to: string
    params?: { taskId?: string }
  }) => <a href={`${to}/${params?.taskId}`}>{children}</a>,
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useDeleteTask: vi.fn(),
}))

// Mock tooltip provider to simplify testing
vi.mock('@/lib/components/ui/tooltip', () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="tooltip-content">{children}</div>
  ),
  TooltipProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

describe('TaskList', () => {
  const mockDeleteMutate = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock global confirm
    global.confirm = vi.fn(() => true)

    // Default mock implementations
    vi.mocked(useRepositoriesList).mockReturnValue(
      createMockQuery([
        { id: 'repo-1', name: 'Test Repository 1' },
        { id: 'repo-2', name: 'Test Repository 2' },
      ]),
    )

    vi.mocked(useDeleteTask).mockReturnValue(createMockMutation({ mutate: mockDeleteMutate }))
  })

  describe('Rendering', () => {
    it('renders loading state', () => {
      render(<TaskList tasks={[]} loading={true} />)

      expect(screen.getByText('Loading tasks...')).toBeInTheDocument()
    })

    it('renders empty state when no tasks', () => {
      render(<TaskList tasks={[]} loading={false} />)

      expect(screen.getByText('No tasks found')).toBeInTheDocument()
    })

    it('renders task list with correct columns', () => {
      const tasks = [createMockTask({ taskId: 'task-1' })]

      render(<TaskList tasks={tasks} />)

      expect(screen.getByText('Task')).toBeInTheDocument()
      expect(screen.getByText('Environment')).toBeInTheDocument()
      expect(screen.getByText('Last Activity')).toBeInTheDocument()
      expect(screen.getByText('Status')).toBeInTheDocument()
      expect(screen.getByText('Git')).toBeInTheDocument()
      expect(screen.getByText('Actions')).toBeInTheDocument()
    })

    it('renders task data correctly', () => {
      const task = createMockTask({
        taskId: 'task-123',
        repositoryId: 'repo-1',
        status: 'running',
        iterations: [
          {
            iterationNumber: 1,
            initialMessage: 'Fix the bug in authentication\nMore details here',
            costUsd: 0.01,
            status: 'completed' as const,
            startTime: new Date(Date.now() - 3600000),
            messageCount: 1,
          },
        ],
        createdAt: new Date(Date.now() - 3600000), // 1 hour ago
      })

      render(<TaskList tasks={[task]} />)

      expect(screen.getByText('Fix the bug in authentication')).toBeInTheDocument()
      expect(screen.getByText('Test Repository 1')).toBeInTheDocument()
      // Time display might vary based on test execution time, check for "ago" suffix
      expect(screen.getByText(/ago$/)).toBeInTheDocument()
    })

    it('uses task ID as fallback when no initial message', () => {
      const task = createMockTask({
        taskId: 'abc12345-def6-7890',
        iterations: [],
      })

      render(<TaskList tasks={[task]} />)

      expect(screen.getByText('Task abc12345')).toBeInTheDocument()
    })

    it('sorts tasks by last activity (newest first)', () => {
      const oldTask = createMockTask({
        taskId: 'old-task',
        iterations: [
          {
            iterationNumber: 1,
            initialMessage: 'Old task',
            costUsd: 0.01,
            status: 'completed' as const,
            startTime: new Date(Date.now() - 7200000),
            messageCount: 1,
          },
        ],
        lastMessageAt: new Date(Date.now() - 7200000), // 2 hours ago
      })

      const newTask = createMockTask({
        taskId: 'new-task',
        iterations: [
          {
            iterationNumber: 1,
            initialMessage: 'New task',
            costUsd: 0.01,
            status: 'completed' as const,
            startTime: new Date(Date.now() - 1800000),
            messageCount: 1,
          },
        ],
        lastMessageAt: new Date(Date.now() - 1800000), // 30 mins ago
      })

      render(<TaskList tasks={[oldTask, newTask]} />)

      const links = screen.getAllByRole('link')
      expect(links[0]).toHaveTextContent('New task')
      expect(links[1]).toHaveTextContent('Old task')
    })
  })

  describe('Status Display', () => {
    it('displays different status icons based on task status', () => {
      const statuses: Array<Task['status']> = ['running', 'open', 'error', 'merged', 'closed']

      statuses.forEach((status) => {
        const task = createMockTask({ status })
        const { rerender } = render(<TaskList tasks={[task]} />)

        // Check that status is displayed
        const statusElements = screen.getAllByText(new RegExp(status, 'i'))
        expect(statusElements.length).toBeGreaterThan(0)

        rerender(<TaskList tasks={[]} />)
      })
    })

    it('shows git sync status for open tasks', () => {
      const task = createMockTask({
        status: 'open',
        iterations: [
          {
            iterationNumber: 1,
            initialMessage: 'Test',
            costUsd: 0.01,
            status: 'completed' as const,
            startTime: new Date(),
            messageCount: 1,
            gitStatus: {
              status: 'synced',
              hasUncommittedChanges: false,
              hasConflicts: false,
              commitsAhead: 0,
              commitsBehind: 0,
              currentBranch: 'test-branch',
              baseBranch: 'main',
              originExists: true,
            },
          },
        ],
      })

      render(<TaskList tasks={[task]} />)

      // Git sync status should be visible
      const syncTexts = screen.getAllByText('In sync with origin')
      expect(syncTexts.length).toBeGreaterThan(0)
    })

    it('does not show git sync status for non-open tasks', () => {
      const task = createMockTask({
        status: 'running',
        iterations: [
          {
            iterationNumber: 1,
            initialMessage: 'Test',
            costUsd: 0.01,
            status: 'completed' as const,
            startTime: new Date(),
            messageCount: 1,
            gitStatus: {
              status: 'synced',
              hasUncommittedChanges: false,
              hasConflicts: false,
              commitsAhead: 0,
              commitsBehind: 0,
              currentBranch: 'test-branch',
              baseBranch: 'main',
              originExists: true,
            },
          },
        ],
      })

      render(<TaskList tasks={[task]} />)

      expect(screen.queryByText('In sync with origin')).not.toBeInTheDocument()
      expect(screen.getByText('-')).toBeInTheDocument()
    })
  })

  describe('Repository Name Resolution', () => {
    it('displays repository name when available', () => {
      const task = createMockTask({ repositoryId: 'repo-2' })

      render(<TaskList tasks={[task]} />)

      expect(screen.getByText('Test Repository 2')).toBeInTheDocument()
    })

    it('falls back to repository ID when name not found', () => {
      const task = createMockTask({ repositoryId: 'unknown-repo' })

      render(<TaskList tasks={[task]} />)

      expect(screen.getByText('unknown-repo')).toBeInTheDocument()
    })
  })

  describe('User Interactions', () => {
    it('navigates to task detail on row click', () => {
      const task = createMockTask({ taskId: 'task-123' })

      render(<TaskList tasks={[task]} />)

      const link = screen.getByRole('link')
      expect(link).toHaveAttribute('href', '/tasks/$taskId/task-123')
    })

    it('handles delete button click', async () => {
      const user = userEvent.setup()
      const task = createMockTask({ taskId: 'task-to-delete' })

      render(<TaskList tasks={[task]} />)

      const deleteButton = screen.getByRole('button')
      await user.click(deleteButton)

      expect(global.confirm).toHaveBeenCalledWith('Are you sure you want to delete this task?')
      expect(mockDeleteMutate).toHaveBeenCalledWith('task-to-delete')
    })

    it('does not delete when user cancels confirmation', async () => {
      const user = userEvent.setup()
      global.confirm = vi.fn(() => false)

      const task = createMockTask({ taskId: 'task-123' })

      render(<TaskList tasks={[task]} />)

      const deleteButton = screen.getByRole('button')
      await user.click(deleteButton)

      expect(mockDeleteMutate).not.toHaveBeenCalled()
    })

    it('disables delete buttons when deletion is pending', () => {
      vi.mocked(useDeleteTask).mockReturnValue(
        createMockMutation({ mutate: mockDeleteMutate, isPending: true }),
      )

      const tasks = [createMockTask({ taskId: 'task-1' }), createMockTask({ taskId: 'task-2' })]

      render(<TaskList tasks={tasks} />)

      const deleteButtons = screen.getAllByRole('button')
      deleteButtons.forEach((button) => {
        expect(button).toBeDisabled()
      })
    })
  })

  describe('Edge Cases', () => {
    it('handles missing repository data gracefully', () => {
      vi.mocked(useRepositoriesList).mockReturnValue(createMockQuery(undefined))

      const task = createMockTask({ repositoryId: 'repo-1' })

      render(<TaskList tasks={[task]} />)

      // Should display repository ID as fallback
      expect(screen.getByText('repo-1')).toBeInTheDocument()
    })

    it('handles tasks with no iterations', () => {
      const task = createMockTask({
        taskId: 'task-no-iterations',
        iterations: undefined,
      })

      render(<TaskList tasks={[task]} />)

      // Task ID is truncated to 8 characters: 'task-no-'
      expect(screen.getByText('Task task-no-')).toBeInTheDocument()
    })

    it('prevents event propagation on delete button click', async () => {
      const task = createMockTask({ taskId: 'task-123' })

      render(<TaskList tasks={[task]} />)

      const deleteButton = screen.getByRole('button')
      const clickEvent = new MouseEvent('click', { bubbles: true })
      const preventDefaultSpy = vi.spyOn(clickEvent, 'preventDefault')
      const stopPropagationSpy = vi.spyOn(clickEvent, 'stopPropagation')

      deleteButton.dispatchEvent(clickEvent)

      await waitFor(() => {
        expect(preventDefaultSpy).toHaveBeenCalled()
        expect(stopPropagationSpy).toHaveBeenCalled()
      })
    })
  })
})
