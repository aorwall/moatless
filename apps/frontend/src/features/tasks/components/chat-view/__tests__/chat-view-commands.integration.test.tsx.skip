import type { Command, TaskIteration } from '@moatless/types'
import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  useEventStream,
  useStreamingStatus,
  useTaskData,
  useWorkspaceStatus,
} from '@/features/tasks/contexts/task-providers'
import { useSelectedIteration } from '@/features/tasks/contexts/task-ui-provider'
import { useSendMessage, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ChatView } from '../chat-view'

// Mock all dependencies
vi.mock('@/features/tasks/contexts/task-providers')
vi.mock('@/features/tasks/contexts/task-ui-provider')
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/tasks/hooks/use-auto-scroll', () => ({
  useAutoScroll: () => ({ scrollRef: { current: null } }),
}))
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task-id' }),
}))

// Mock components that aren't relevant to command testing
vi.mock('../chat-header', () => ({
  ChatHeader: () => null,
}))
vi.mock('../chat-view-skeleton', () => ({
  ChatViewSkeleton: () => null,
}))
vi.mock('../../event-viewer', () => ({
  TaskMinifiedLastEventsDisplay: () => null,
}))
vi.mock('../../chat-input', () => ({
  ChatInput: () => null,
}))

describe('ChatView Commands Integration', () => {
  const mockSendMessage = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock scrollIntoView
    Element.prototype.scrollIntoView = vi.fn()

    // Default mocks
    vi.mocked(useTaskData).mockReturnValue({
      taskId: 'test-task-id',
      status: 'open',
      iterations: [],
      repositoryId: 'test-repo',
      isLoading: false,
    } as any)

    vi.mocked(useEventStream).mockReturnValue({
      events: [],
    } as any)

    vi.mocked(useStreamingStatus).mockReturnValue({
      isStreaming: false,
    } as any)

    vi.mocked(useSelectedIteration).mockReturnValue({
      setSelectedIteration: vi.fn(),
    } as any)

    vi.mocked(useSendMessage).mockReturnValue({
      mutate: mockSendMessage,
      isPending: false,
    } as any)

    vi.mocked(useWorkspaceStatus).mockReturnValue({
      workspaceHasChanges: false,
    } as any)

    vi.mocked(useTaskCommands).mockReturnValue({
      data: { commands: [] },
      isLoading: false,
    } as any)
  })

  describe('Command visibility based on workspace status', () => {
    it('should show commit command when there are uncommitted changes', async () => {
      const commitCommand: Command = {
        name: 'commit',
        description: 'Commit changes',
        prompt: 'commit the changes',
        conditions: {
          requiresUncommittedChanges: true,
          requiresTaskNotRunning: true,
        },
        icon: 'GitCommit',
      }

      vi.mocked(useWorkspaceStatus).mockReturnValue({
        workspaceHasChanges: true,
      } as any)

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: [commitCommand] },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.getByText('Commit changes')).toBeInTheDocument()
      })
    })

    it('should hide commit command when there are no uncommitted changes', async () => {
      // When there are no uncommitted changes, the backend should not return commit command
      vi.mocked(useWorkspaceStatus).mockReturnValue({
        workspaceHasChanges: false,
      } as any)

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: [] }, // Backend filters out commit command
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.queryByText('Commit changes')).not.toBeInTheDocument()
      })
    })
  })

  describe('Command visibility based on task status', () => {
    it('should hide all commands when task is running', async () => {
      vi.mocked(useTaskData).mockReturnValue({
        taskId: 'test-task-id',
        status: 'running',
        iterations: [],
        repositoryId: 'test-repo',
        isLoading: false,
      } as any)

      // The component filters out commands when task is running (line 209 in chat-view.tsx)
      vi.mocked(useTaskCommands).mockReturnValue({
        data: {
          commands: [
            {
              name: 'commit',
              description: 'Commit changes',
              prompt: 'commit',
            },
            {
              name: 'continue',
              description: 'Continue',
              prompt: 'continue',
            },
          ],
        },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        // Commands are not rendered when task is running
        expect(screen.queryByText('Commit changes')).not.toBeInTheDocument()
        expect(screen.queryByText('Continue')).not.toBeInTheDocument()
      })
    })

    it('should show continue command after error', async () => {
      const continueCommand: Command = {
        name: 'continue',
        description: 'Continue after error',
        prompt: 'continue',
        conditions: {
          requiresLastIterationError: true,
        },
        icon: 'Play',
      }

      const errorIteration: TaskIteration = {
        iterationNumber: 1,
        status: 'error',
        initialMessage: 'Test message',
        error: { message: 'Test error' },
        messageCount: 1,
        costUsd: 0.01,
      } as TaskIteration

      vi.mocked(useTaskData).mockReturnValue({
        taskId: 'test-task-id',
        status: 'error',
        iterations: [errorIteration],
        repositoryId: 'test-repo',
        isLoading: false,
      } as any)

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: [continueCommand] },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.getByText('Continue after error')).toBeInTheDocument()
      })
    })
  })

  describe('Command visibility based on git sync status', () => {
    it('should show create-pr command when branch is ahead', async () => {
      const createPrCommand: Command = {
        name: 'create-pr',
        description: 'Create pull request',
        prompt: 'create pr',
        conditions: {
          requiresAheadOfMain: true,
          allowedGitSyncStatuses: ['ahead'],
        },
        icon: 'GitPullRequest',
      }

      const aheadIteration: TaskIteration = {
        iterationNumber: 1,
        status: 'completed',
        initialMessage: 'Test',
        messageCount: 1,
        costUsd: 0.01,
        startTime: new Date(),
        gitStatus: {
          status: 'ahead',
          hasUncommittedChanges: false,
          hasConflicts: false,
          commitsAhead: 2,
          commitsBehind: 0,
          currentBranch: 'feature',
          baseBranch: 'main',
          originExists: true,
        },
      }

      vi.mocked(useTaskData).mockReturnValue({
        taskId: 'test-task-id',
        status: 'open',
        iterations: [aheadIteration],
        repositoryId: 'test-repo',
        isLoading: false,
      } as any)

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: [createPrCommand] },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.getByText('Create pull request')).toBeInTheDocument()
      })
    })

    it('should show sync command when branch is behind', async () => {
      const syncCommand: Command = {
        name: 'sync-with-main',
        description: 'Sync with main',
        prompt: 'sync',
        conditions: {
          requiresBehindMain: true,
          allowedGitSyncStatuses: ['behind', 'diverged'],
        },
        icon: 'GitBranch',
      }

      const behindIteration: TaskIteration = {
        iterationNumber: 1,
        status: 'completed',
        initialMessage: 'Test',
        messageCount: 1,
        costUsd: 0.01,
        startTime: new Date(),
        gitStatus: {
          status: 'behind',
          hasUncommittedChanges: false,
          hasConflicts: false,
          commitsAhead: 0,
          commitsBehind: 3,
          currentBranch: 'feature',
          baseBranch: 'main',
          originExists: true,
        },
      }

      vi.mocked(useTaskData).mockReturnValue({
        taskId: 'test-task-id',
        status: 'open',
        iterations: [behindIteration],
        repositoryId: 'test-repo',
        isLoading: false,
      } as any)

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: [syncCommand] },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.getByText('Sync with main')).toBeInTheDocument()
      })
    })
  })

  describe('Multiple commands with different conditions', () => {
    it('should show only commands that meet their conditions', async () => {
      const commands: Command[] = [
        {
          name: 'commit',
          description: 'Commit changes',
          prompt: 'commit',
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'continue',
          description: 'Continue after error',
          prompt: 'continue',
          conditions: {
            requiresLastIterationError: true,
          },
        },
        {
          name: 'always-show',
          description: 'Always available',
          prompt: 'always',
        },
      ]

      vi.mocked(useWorkspaceStatus).mockReturnValue({
        workspaceHasChanges: true,
      } as any)

      vi.mocked(useTaskData).mockReturnValue({
        taskId: 'test-task-id',
        status: 'open',
        iterations: [
          {
            iterationNumber: 1,
            status: 'completed',
            initialMessage: 'Test',
            messageCount: 1,
            costUsd: 0.01,
          } as TaskIteration,
        ],
        repositoryId: 'test-repo',
        isLoading: false,
      } as any)

      // Only mock commands that should be shown based on conditions
      // In a real scenario, the API would filter these
      const filteredCommands = commands.filter((cmd) => {
        if (cmd.conditions?.requiresLastIterationError) {
          // Last iteration is completed, not error
          return false
        }
        if (cmd.conditions?.requiresUncommittedChanges) {
          // We have uncommitted changes
          return true
        }
        // Commands with no conditions are always shown
        return !cmd.conditions
      })

      vi.mocked(useTaskCommands).mockReturnValue({
        data: { commands: filteredCommands },
        isLoading: false,
      } as any)

      render(<ChatView />)

      await waitFor(() => {
        expect(screen.getByText('Commit changes')).toBeInTheDocument()
        expect(screen.queryByText('Continue after error')).not.toBeInTheDocument()
        expect(screen.getByText('Always available')).toBeInTheDocument()
      })
    })
  })
})
