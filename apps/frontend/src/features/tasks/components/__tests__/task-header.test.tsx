import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { toast } from 'sonner'
import { TaskHeader } from '../task-header'
import { useStopAgent } from '../../hooks/use-tasks'
import { useServerStatus, useStartServer, useStopServer } from '@/features/preview'
import {
  useStreamingStatus,
  useTaskData,
  useTaskStatus,
} from '@/features/tasks/contexts/task-providers'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task-123' }),
}))

vi.mock('sonner', () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useStopAgent: vi.fn(),
}))

vi.mock('@/features/preview', () => ({
  useServerStatus: vi.fn(),
  useStartServer: vi.fn(),
  useStopServer: vi.fn(),
}))

vi.mock('@/features/tasks/contexts/task-providers', () => ({
  useTaskStatus: vi.fn(),
  useTaskData: vi.fn(),
  useStreamingStatus: vi.fn(),
}))

// Mock sidebar trigger to avoid implementation details
vi.mock('@/lib/components/ui/sidebar.tsx', () => ({
  SidebarTrigger: () => <button data-testid="sidebar-trigger">Menu</button>,
}))

describe('TaskHeader', () => {
  const mockStopAgent = vi.fn()
  const mockStartServer = vi.fn()
  const mockStopServer = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Default mock implementations
    vi.mocked(useTaskStatus).mockReturnValue({
      status: 'running',
    } as any)

    vi.mocked(useTaskData).mockReturnValue({
      agentType: 'claude-code',
      createdAt: '2025-01-01T00:00:00Z',
      iterations: [
        { costUsd: 0.01 },
        { costUsd: 0.02 },
      ],
    } as any)

    vi.mocked(useStreamingStatus).mockReturnValue({
      isStreaming: true,
    } as any)

    vi.mocked(useServerStatus).mockReturnValue({
      data: null,
      isLoading: false,
    } as any)

    vi.mocked(useStopAgent).mockReturnValue({
      mutate: mockStopAgent,
      isPending: false,
    } as any)

    vi.mocked(useStartServer).mockReturnValue({
      mutate: mockStartServer,
      isPending: false,
    } as any)

    vi.mocked(useStopServer).mockReturnValue({
      mutate: mockStopServer,
      isPending: false,
    } as any)
  })

  describe('Rendering', () => {
    it('renders task information correctly', () => {
      render(<TaskHeader />)

      expect(screen.getByText('Task test-tas')).toBeInTheDocument() // First 8 chars of taskId
      expect(screen.getByText('claude-code')).toBeInTheDocument()
      expect(screen.getByText('running')).toBeInTheDocument()
      expect(screen.getByText('2 turns')).toBeInTheDocument()
      expect(screen.getByText('$0.0300')).toBeInTheDocument()
    })

    it('renders nothing when status is not available', () => {
      vi.mocked(useTaskStatus).mockReturnValue({
        status: null,
      } as any)

      const { container } = render(<TaskHeader />)
      expect(container.firstChild).toBeNull()
    })

    it('shows different status badges based on task status', () => {
      const statuses = ['open', 'running', 'error', 'closed', 'merged']

      statuses.forEach((status) => {
        vi.mocked(useTaskStatus).mockReturnValue({ status } as any)
        const { rerender } = render(<TaskHeader />)
        
        const badge = screen.getByText(status)
        expect(badge).toBeInTheDocument()
        expect(badge.className).toContain('capitalize')
        
        rerender(<TaskHeader />)
      })
    })

    it('calculates total cost correctly from iterations', () => {
      vi.mocked(useTaskData).mockReturnValue({
        agentType: 'claude-code',
        createdAt: '2025-01-01T00:00:00Z',
        iterations: [
          { costUsd: 0.123 },
          { costUsd: 0.456 },
          { costUsd: 0.789 },
        ],
      } as any)

      render(<TaskHeader />)

      expect(screen.getByText('$1.3680')).toBeInTheDocument()
    })
  })

  describe('User Interactions', () => {
    it('handles stop agent button click', async () => {
      const user = userEvent.setup()
      render(<TaskHeader />)

      const stopButton = screen.getByRole('button', { name: /stop agent/i })
      await user.click(stopButton)

      expect(mockStopAgent).toHaveBeenCalled()
    })

    it('shows loading state when stopping agent', () => {
      vi.mocked(useStopAgent).mockReturnValue({
        mutate: mockStopAgent,
        isPending: true,
      } as any)

      render(<TaskHeader />)

      expect(screen.getByText('Stopping...')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /stopping/i })).toBeDisabled()
    })

    it('shows toast on successful agent stop', async () => {
      const user = userEvent.setup()
      
      // Mock the onSuccess callback
      vi.mocked(useStopAgent).mockImplementation((_taskId, options) => {
        return {
          mutate: () => {
            mockStopAgent()
            options?.onSuccess?.({} as any, {} as any, {} as any)
          },
          isPending: false,
        } as any
      })

      render(<TaskHeader />)

      const stopButton = screen.getByRole('button', { name: /stop agent/i })
      await user.click(stopButton)

      expect(toast.success).toHaveBeenCalledWith('Agent stopped successfully')
    })

    it('shows toast on agent stop error', async () => {
      const user = userEvent.setup()
      const error = new Error('Network error')
      
      vi.mocked(useStopAgent).mockImplementation((_taskId, options) => {
        return {
          mutate: () => {
            mockStopAgent()
            options?.onError?.(error, {} as any, {} as any)
          },
          isPending: false,
        } as any
      })

      render(<TaskHeader />)

      const stopButton = screen.getByRole('button', { name: /stop agent/i })
      await user.click(stopButton)

      expect(toast.error).toHaveBeenCalledWith('Failed to stop agent: Network error')
    })
  })

  describe('Server Controls', () => {
    it('shows start server button when server is configured but not running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: {
          configured: true,
          running: false,
        },
        isLoading: false,
      } as any)

      render(<TaskHeader />)

      expect(screen.getByRole('button', { name: /start server/i })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: /stop server/i })).not.toBeInTheDocument()
    })

    it('shows stop server button when server is running', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: {
          configured: true,
          running: true,
          url: 'http://localhost:3000',
          pid: 1234,
          uptimeSeconds: 120,
        },
        isLoading: false,
      } as any)

      render(<TaskHeader />)

      expect(screen.getByRole('button', { name: /stop server/i })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: /start server/i })).not.toBeInTheDocument()
    })

    it('handles start server action', async () => {
      const user = userEvent.setup()
      
      vi.mocked(useServerStatus).mockReturnValue({
        data: { configured: true, running: false },
        isLoading: false,
      } as any)

      vi.mocked(useStartServer).mockImplementation((options) => {
        return {
          mutate: (taskId: string) => {
            mockStartServer(taskId)
            options?.onSuccess?.({} as any, {} as any, {} as any)
          },
          isPending: false,
        } as any
      })

      render(<TaskHeader />)

      const startButton = screen.getByRole('button', { name: /start server/i })
      await user.click(startButton)

      expect(mockStartServer).toHaveBeenCalledWith('test-task-123')
      expect(toast.success).toHaveBeenCalledWith('Server started successfully')
    })

    it('disables server buttons when loading', () => {
      vi.mocked(useServerStatus).mockReturnValue({
        data: { configured: true, running: false },
        isLoading: true,
      } as any)

      render(<TaskHeader />)

      expect(screen.getByRole('button', { name: /start server/i })).toBeDisabled()
    })
  })

  describe('Streaming Status', () => {
    it('shows not streaming warning when task is running but not streaming', () => {
      vi.mocked(useStreamingStatus).mockReturnValue({
        isStreaming: false,
      } as any)

      render(<TaskHeader />)

      expect(screen.getByText('Not streaming')).toBeInTheDocument()
      const warningIcon = document.querySelector('.lucide-wifi-off')
      expect(warningIcon).toBeInTheDocument()
    })

    it('does not show streaming warning when streaming is active', () => {
      vi.mocked(useStreamingStatus).mockReturnValue({
        isStreaming: true,
      } as any)

      render(<TaskHeader />)

      expect(screen.queryByText('Not streaming')).not.toBeInTheDocument()
    })

    it('does not show streaming warning when task is not running', () => {
      vi.mocked(useTaskStatus).mockReturnValue({
        status: 'closed',
      } as any)

      vi.mocked(useStreamingStatus).mockReturnValue({
        isStreaming: false,
      } as any)

      render(<TaskHeader />)

      expect(screen.queryByText('Not streaming')).not.toBeInTheDocument()
    })
  })

  describe('Edge Cases', () => {
    it('handles missing createdAt gracefully', () => {
      vi.mocked(useTaskData).mockReturnValue({
        agentType: 'claude-code',
        createdAt: null,
        iterations: [],
      } as any)

      render(<TaskHeader />)

      // Should not crash and should show empty string for date
      expect(screen.getByText('claude-code')).toBeInTheDocument()
    })

    it('handles empty iterations array', () => {
      vi.mocked(useTaskData).mockReturnValue({
        agentType: 'claude-code',
        createdAt: '2025-01-01T00:00:00Z',
        iterations: [],
      } as any)

      render(<TaskHeader />)

      expect(screen.getByText('0 turns')).toBeInTheDocument()
      expect(screen.getByText('$0.0000')).toBeInTheDocument()
    })

    it('does not show stop button when task is not running', () => {
      vi.mocked(useTaskStatus).mockReturnValue({
        status: 'closed',
      } as any)

      render(<TaskHeader />)

      expect(screen.queryByRole('button', { name: /stop agent/i })).not.toBeInTheDocument()
    })
  })
})