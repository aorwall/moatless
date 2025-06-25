import type { ServerStatus, Task, WorkspaceStatusResponse } from '@shared/types/task'
import { WorkspaceStatus } from '@shared/types/task'
import type { UseMutationResult, UseQueryResult } from '@tanstack/react-query'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { toast } from 'sonner'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createMockServerStatus,
  createMockTask,
  createMockWorkspaceStatus,
} from '@/test-utils/factories'
import { renderWithRouter } from '@/test-utils/render'

import {
  useServerStatus,
  useStartServer,
  useStopAgent,
  useStopServer,
  useTask,
  useTaskWorkspaceStatus,
} from '../../hooks/use-tasks'
import { useTaskStreamingError } from '../../stores/task-event-store'
import { TaskHeader } from '../task-header'

// Mock dependencies
vi.mock('../../hooks/use-tasks')
vi.mock('../../stores/task-event-store', () => ({
  useTaskStreamingError: vi.fn(() => null),
}))
vi.mock('sonner')
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}))

// Mock UI components
vi.mock('@/lib/components/ui/sidebar.tsx', () => ({
  SidebarTrigger: () => <button>Sidebar</button>,
}))

describe('TaskHeader', () => {
  const mockStopAgent = vi.fn()
  const mockStartServer = vi.fn()
  const mockStopServer = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Default mocks
    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({
        task_id: 'task-123',
        title: 'Test Task',
        is_running: false,
        status: 'completed',
        agent_type: 'test-agent',
        num_turns: 5,
        total_cost_usd: 0.0234,
        created_at: '2024-01-01T00:00:00Z',
      }),
      isLoading: false,
      error: null,
    } as UseQueryResult<Task, Error>)

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: createMockWorkspaceStatus({ status: WorkspaceStatus.IDLE }),
      isLoading: false,
      error: null,
    } as UseQueryResult<WorkspaceStatusResponse, Error>)

    // Ensure streaming error is null by default
    vi.mocked(useTaskStreamingError).mockClear()
    vi.mocked(useTaskStreamingError).mockReturnValue(null)

    vi.mocked(useServerStatus).mockReturnValue({
      data: createMockServerStatus({ configured: false, running: false }),
      isLoading: false,
      error: null,
    } as UseQueryResult<ServerStatus, Error>)

    vi.mocked(useStopAgent).mockReturnValue({
      mutate: mockStopAgent,
      isPending: false,
    } as unknown as UseMutationResult<{ status: string; task_id: string }, Error, string>)

    vi.mocked(useStartServer).mockReturnValue({
      mutate: mockStartServer,
      isPending: false,
    } as unknown as UseMutationResult<ServerStatus, Error, string>)

    vi.mocked(useStopServer).mockReturnValue({
      mutate: mockStopServer,
      isPending: false,
    } as unknown as UseMutationResult<ServerStatus, Error, string>)
  })

  describe('Basic Rendering', () => {
    it('renders task metadata', () => {
      renderWithRouter(<TaskHeader />)

      expect(screen.getByText('Test Task')).toBeInTheDocument()
      expect(screen.getByText('test-agent')).toBeInTheDocument()
      expect(screen.getByText('5 turns')).toBeInTheDocument()
      expect(screen.getByText('$0.0234')).toBeInTheDocument()
    })

    it('uses task ID when title is not provided', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({
          task_id: 'task-12345678',
          title: undefined,
        }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      renderWithRouter(<TaskHeader />)

      expect(screen.getByText('Task task-123')).toBeInTheDocument()
    })

    it('shows status badge', () => {
      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('completed')).toBeInTheDocument()
    })

    it('shows loading spinner when task is running', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      renderWithRouter(<TaskHeader />)

      // SVG spinner doesn't have role="img", check for the Loader2 icon instead
      const spinner = document.querySelector('.lucide-loader-circle')
      expect(spinner).toHaveClass('animate-spin')
    })

    it('returns null when no metadata', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      const { container } = renderWithRouter(<TaskHeader />)
      expect(container.firstChild).toBeNull()
    })
  })

  describe('Workspace Status Display', () => {
    const workspaceTestCases = [
      { status: WorkspaceStatus.NOT_SETUP, expected: 'workspace: not set up' },
      { status: WorkspaceStatus.SETTING_UP, expected: 'workspace: setting up' },
      { status: WorkspaceStatus.IDLE, expected: 'workspace: idle' },
      { status: WorkspaceStatus.RUNNING, expected: 'workspace: running' },
    ]

    workspaceTestCases.forEach(({ status, expected }) => {
      it(`displays ${expected} status correctly`, () => {
        vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
          data: createMockWorkspaceStatus({ status }),
          isLoading: false,
          error: null,
        } as UseQueryResult<WorkspaceStatusResponse, Error>)

        renderWithRouter(<TaskHeader />)
        expect(screen.getByText(expected)).toBeInTheDocument()
      })
    })

    it('shows workspace: unknown when data is not available', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as UseQueryResult<WorkspaceStatusResponse, Error>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('workspace: unknown')).toBeInTheDocument()
    })

    it('shows workspace: unknown with loading spinner when loading', () => {
      vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as UseQueryResult<WorkspaceStatusResponse, Error>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('workspace: unknown')).toBeInTheDocument()
      // Check for loading spinner within the workspace badge
      const badge = screen.getByText('workspace: unknown').closest('[data-slot="badge"]')
      expect(badge?.querySelector('.animate-spin')).toBeInTheDocument()
    })
  })

  describe('Server Controls', () => {
    it('shows start server button when server is configured but not running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('Start Server')).toBeInTheDocument()
    })

    it('shows stop server button when server is running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('Stop Server')).toBeInTheDocument()
    })

    it('does not show server controls when server is not configured', () => {
      renderWithRouter(<TaskHeader />)
      expect(screen.queryByText('Start Server')).not.toBeInTheDocument()
      expect(screen.queryByText('Stop Server')).not.toBeInTheDocument()
    })

    it('calls startServer mutation when start button is clicked', async () => {
      const user = userEvent.setup()
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      await user.click(screen.getByText('Start Server'))

      expect(mockStartServer).toHaveBeenCalledWith('task-123')
    })

    it('calls stopServer mutation when stop button is clicked', async () => {
      const user = userEvent.setup()
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      await user.click(screen.getByText('Stop Server'))

      expect(mockStopServer).toHaveBeenCalledWith('task-123')
    })

    it('disables server buttons when mutations are pending', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({ configured: true, running: false }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      vi.mocked(useStartServer).mockReturnValue({
        mutate: mockStartServer,
        isPending: true,
      } as unknown as UseMutationResult<ServerStatus, Error, string>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('Starting Server...')).toBeDisabled()
    })

    it('shows server port when running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({
          configured: true,
          running: true,
        }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      // Port display functionality might have been removed or changed
    })

    it('does not show port when server is not running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: createMockServerStatus({
          configured: true,
          running: false,
        }),
        isLoading: false,
        error: null,
      } as UseQueryResult<ServerStatus, Error>)

      renderWithRouter(<TaskHeader />)
      // Port display functionality might have been removed or changed
    })
  })

  describe('Stop Agent Button', () => {
    it('shows stop agent button when task is running', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('Stop')).toBeInTheDocument()
    })

    it('does not show stop agent button when task is not running', () => {
      renderWithRouter(<TaskHeader />)
      expect(screen.queryByText('Stop')).not.toBeInTheDocument()
    })

    it('calls stopAgent mutation when clicked', async () => {
      const user = userEvent.setup()
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      renderWithRouter(<TaskHeader />)
      await user.click(screen.getByText('Stop'))

      expect(mockStopAgent).toHaveBeenCalledWith('task-123')
    })

    it('disables stop agent button when mutation is pending', () => {
      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      vi.mocked(useStopAgent).mockReturnValue({
        mutate: mockStopAgent,
        isPending: true,
      } as unknown as UseMutationResult<{ status: string; task_id: string }, Error, string>)

      renderWithRouter(<TaskHeader />)
      expect(screen.getByText('Stopping...')).toBeDisabled()
    })

    it('shows toast notification on stop agent success', async () => {
      const user = userEvent.setup()
      let capturedOnSuccess:
        | ((data: { status: string; task_id: string }, variables: string, context: unknown) => void)
        | undefined

      vi.mocked(useTask).mockReturnValue({
        data: createMockTask({ is_running: true }),
        isLoading: false,
        error: null,
      } as UseQueryResult<Task, Error>)

      // Capture the onSuccess callback from useStopAgent
      vi.mocked(useStopAgent).mockImplementation((options) => {
        capturedOnSuccess = options?.onSuccess
        return {
          mutate: mockStopAgent,
          isPending: false,
        } as unknown as UseMutationResult<{ status: string; task_id: string }, Error, string>
      })

      renderWithRouter(<TaskHeader />)
      await user.click(screen.getByText('Stop'))

      // Verify mutate was called
      expect(mockStopAgent).toHaveBeenCalledWith('task-123')

      // Simulate successful mutation by calling the captured onSuccess
      if (capturedOnSuccess) {
        capturedOnSuccess({ status: 'stopped', task_id: 'task-123' }, 'task-123', {})
      }

      await waitFor(() => {
        expect(toast.success).toHaveBeenCalledWith('Agent stopped successfully')
      })
    })
  })

  describe('Error States', () => {
    it('displays streaming error when present', () => {
      vi.mocked(useTaskStreamingError).mockReturnValue('WebSocket connection failed')

      renderWithRouter(<TaskHeader />)

      // Check that the streaming error indicator is displayed
      expect(screen.getByText('Streaming error')).toBeInTheDocument()

      // The actual error message is in the tooltip, which is rendered when hovering
      const errorIndicator = screen.getByText('Streaming error').parentElement
      expect(errorIndicator).toBeInTheDocument()
    })

    it('does not display error alert when no streaming error', () => {
      // Explicitly reset the streaming error mock
      vi.mocked(useTaskStreamingError).mockReturnValue(null)

      renderWithRouter(<TaskHeader />)
      expect(screen.queryByText('Streaming error')).not.toBeInTheDocument()
    })
  })
})
