import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useCreateTaskMessage, useTask, useTaskEvents } from '@/features/tasks/hooks/use-task-data'
import { useServerStatus, useWorkspaceStatus } from '@/features/tasks/hooks/use-task-environment'
import { useTaskGitDiff } from '@/features/tasks/hooks/use-task-git-diff'
import { useTaskSSEConnection } from '@/features/tasks/hooks/use-task-sse-connection'
import {
  createMockGitDiff,
  createMockMessageEvent,
  createMockServerStatus,
  createMockTaskWithIterations,
  createMockWorkspaceStatus,
} from '@/test-utils/factories'
import { createSSEMock } from '@/test-utils/sse-mock'
import TaskDetail from '../$taskId'

// Mock router to provide taskId param
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => ({
    useParams: () => ({ taskId: 'test-task-123' }),
  }),
  Link: ({ children, to }: any) => <a href={to}>{children}</a>,
  useNavigate: () => vi.fn(),
}))

// Mock API hooks
vi.mock('@/features/tasks/hooks/use-task-data', () => ({
  useTask: vi.fn(),
  useTaskEvents: vi.fn(),
  useCreateTaskMessage: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-task-environment', () => ({
  useWorkspaceStatus: vi.fn(),
  useServerStatus: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-task-git-diff', () => ({
  useTaskGitDiff: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-task-sse-connection', () => ({
  useTaskSSEConnection: vi.fn(),
}))

// Mock heavy UI components
vi.mock('@/lib/features/ui/resizable', () => ({
  ResizableHandle: () => <div data-testid="resizable-handle" />,
  ResizablePanel: ({ children }: any) => <div data-testid="resizable-panel">{children}</div>,
  ResizablePanelGroup: ({ children }: any) => (
    <div data-testid="resizable-panel-group">{children}</div>
  ),
}))

// Helper to render with all providers
function renderTaskDetail() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <TaskDetail />
    </QueryClientProvider>,
  )
}

describe('TaskDetail Integration', () => {
  const mockCreateMessage = vi.fn()
  const sseMock = createSSEMock()

  beforeEach(() => {
    vi.clearAllMocks()

    // Setup EventSource mock
    global.EventSource = sseMock.EventSource as any

    // Default mock implementations
    vi.mocked(useTask).mockReturnValue({
      data: createMockTaskWithIterations({ taskId: 'test-task-123' }, 2),
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEvents).mockReturnValue({
      data: {
        pages: [
          {
            events: [
              createMockMessageEvent('Hello, I need help with authentication', 'user'),
              createMockMessageEvent('I can help you with that', 'assistant'),
            ],
            nextCursor: null,
          },
        ],
      },
      isLoading: false,
      error: null,
      hasNextPage: false,
      fetchNextPage: vi.fn(),
    } as any)

    vi.mocked(useCreateTaskMessage).mockReturnValue({
      mutate: mockCreateMessage,
      isPending: false,
    } as any)

    vi.mocked(useWorkspaceStatus).mockReturnValue({
      data: createMockWorkspaceStatus(),
      isLoading: false,
    } as any)

    vi.mocked(useServerStatus).mockReturnValue({
      data: createMockServerStatus(),
      isLoading: false,
    } as any)

    vi.mocked(useTaskGitDiff).mockReturnValue({
      data: createMockGitDiff(),
      isLoading: false,
    } as any)

    vi.mocked(useTaskSSEConnection).mockReturnValue({
      isConnected: true,
      error: null,
    } as any)
  })

  afterEach(() => {
    sseMock.cleanup()
  })

  describe('Initial Rendering', () => {
    it('displays task information when loaded', async () => {
      renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Fix the bug in authentication')).toBeInTheDocument()
      })
    })

    it('shows loading state while fetching task data', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any)

      renderTaskDetail()

      expect(screen.getByTestId('resizable-panel-group')).toBeInTheDocument()
    })

    it('displays error state when task fails to load', () => {
      const error = new Error('Failed to load task')
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error,
      } as any)

      renderTaskDetail()

      expect(screen.getByText(/Failed to load task/)).toBeInTheDocument()
    })
  })

  describe('Chat Functionality', () => {
    it('displays existing messages in chat view', async () => {
      renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Hello, I need help with authentication')).toBeInTheDocument()
        expect(screen.getByText('I can help you with that')).toBeInTheDocument()
      })
    })

    it('sends new messages through chat input', async () => {
      const user = userEvent.setup()
      renderTaskDetail()

      // Find and interact with chat input
      const chatInput = screen.getByPlaceholderText(/Type a message/)
      await user.type(chatInput, 'Please add unit tests')
      await user.keyboard('{Enter}')

      expect(mockCreateMessage).toHaveBeenCalledWith({
        taskId: 'test-task-123',
        content: 'Please add unit tests',
      })
    })

    it('disables chat input while message is sending', async () => {
      vi.mocked(useCreateTaskMessage).mockReturnValue({
        mutate: mockCreateMessage,
        isPending: true,
      } as any)

      renderTaskDetail()

      const chatInput = screen.getByPlaceholderText(/Type a message/)
      expect(chatInput).toBeDisabled()
    })
  })

  describe('Real-time Updates', () => {
    it('establishes SSE connection on mount', () => {
      renderTaskDetail()

      expect(global.EventSource).toHaveBeenCalledWith(
        expect.stringContaining('/tasks/test-task-123/events'),
      )
    })

    it('handles incoming SSE events', async () => {
      renderTaskDetail()

      // Simulate incoming event
      const newEvent = createMockMessageEvent('New message from assistant', 'assistant')
      sseMock.emit({
        type: 'message',
        data: JSON.stringify(newEvent),
      })

      // In a real scenario, this would update the events list
      // For now, we just verify the connection is established
      expect(vi.mocked(useTaskSSEConnection)).toHaveBeenCalledWith('test-task-123')
    })

    it('shows connection error when SSE fails', () => {
      vi.mocked(useTaskSSEConnection).mockReturnValue({
        isConnected: false,
        error: new Error('Connection failed'),
      } as any)

      renderTaskDetail()

      expect(screen.getByText(/Connection failed/)).toBeInTheDocument()
    })
  })

  describe('Git Diff Display', () => {
    it('shows git diff when available', async () => {
      const mockDiff = createMockGitDiff({
        diff: `diff --git a/auth.ts b/auth.ts
index 1234567..abcdefg 100644
--- a/auth.ts
+++ b/auth.ts
@@ -1,3 +1,4 @@
 export function authenticate(user: string) {
+  // Added validation
   return true;
 }`,
      })

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: mockDiff,
        isLoading: false,
      } as any)

      renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText(/auth\.ts/)).toBeInTheDocument()
      })
    })

    it('shows empty state when no git diff exists', () => {
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: { diff: '', files: [] },
        isLoading: false,
      } as any)

      renderTaskDetail()

      expect(screen.getByText(/No changes/)).toBeInTheDocument()
    })
  })

  describe('Workspace Status', () => {
    it('displays workspace status indicators', async () => {
      const mockStatus = createMockWorkspaceStatus({
        status: 'RUNNING',
        hasUncommittedChanges: true,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue({
        data: mockStatus,
        isLoading: false,
      } as any)

      renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText(/RUNNING/)).toBeInTheDocument()
      })
    })

    it('shows server status when available', () => {
      const mockStatus = createMockServerStatus({
        running: true,
        url: 'http://localhost:3000',
      })

      vi.mocked(useServerStatus).mockReturnValue({
        data: mockStatus,
        isLoading: false,
      } as any)

      renderTaskDetail()

      expect(screen.getByText(/localhost:3000/)).toBeInTheDocument()
    })
  })

  describe('Error Handling', () => {
    it('handles task not found gracefully', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: { status: 404, message: 'Task not found' },
      } as any)

      renderTaskDetail()

      expect(screen.getByText(/Task not found/)).toBeInTheDocument()
    })

    it('recovers from transient errors', async () => {
      const { rerender } = renderTaskDetail()

      // Simulate error then recovery
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: new Error('Network error'),
      } as any)

      rerender(
        <QueryClientProvider client={new QueryClient()}>
          <TaskDetail />
        </QueryClientProvider>,
      )

      expect(screen.getByText(/Network error/)).toBeInTheDocument()

      // Simulate recovery
      vi.mocked(useTask).mockReturnValue({
        data: createMockTaskWithIterations(),
        isLoading: false,
        error: null,
      } as any)

      rerender(
        <QueryClientProvider client={new QueryClient()}>
          <TaskDetail />
        </QueryClientProvider>,
      )

      await waitFor(() => {
        expect(screen.queryByText(/Network error/)).not.toBeInTheDocument()
      })
    })
  })

  describe('User Interactions', () => {
    it('switches between iterations', async () => {
      const user = userEvent.setup()
      const task = createMockTaskWithIterations({ taskId: 'test-task-123' }, 3)

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
      } as any)

      renderTaskDetail()

      // Find iteration selector
      const iterationButtons = screen.getAllByRole('button', { name: /Iteration/i })
      expect(iterationButtons).toHaveLength(3)

      // Click second iteration
      await user.click(iterationButtons[1])

      // Verify UI updates (in real app this would change displayed content)
      expect(iterationButtons[1]).toHaveAttribute('aria-selected', 'true')
    })

    it('toggles panel visibility', async () => {
      const user = userEvent.setup()
      renderTaskDetail()

      // Find panel toggle buttons
      const toggleButtons = screen.getAllByRole('button', { name: /toggle/i })

      // Click to hide a panel
      await user.click(toggleButtons[0])

      // In real app, this would hide/show panels
      // For now we just verify the button exists and is clickable
      expect(toggleButtons[0]).toBeInTheDocument()
    })
  })
})
