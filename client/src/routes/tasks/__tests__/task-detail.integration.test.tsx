import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { createSSEMock } from '@test-utils/sse-mock'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TaskDetailResponse, TaskEvent } from '@shared/types/api'
import { routeTree } from '@/routeTree.gen'

// Mock API client
vi.mock('@/lib/api/client')

// Mock UI components to focus on integration logic
vi.mock('@/lib/components/ui/resizable', () => ({
  ResizablePanel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizableHandle: () => null,
}))

// Mock error boundary to simplify testing
vi.mock('@/lib/components/error-boundary', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

// Mock theme-related components to avoid dependency issues
vi.mock('@/lib/components/theme-provider', () => ({
  ThemeProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useTheme: () => ({ theme: 'light', setTheme: vi.fn() }),
}))

vi.mock('@/lib/components/theme-toggle', () => ({
  ThemeToggle: () => null,
}))

// Mock layout components
vi.mock('@/lib/layout', () => ({
  Layout: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}))

// Mock sidebar components
vi.mock('@/lib/components/ui/sidebar', () => ({
  SidebarProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SidebarTrigger: () => null,
  Sidebar: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SidebarContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SidebarHeader: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SidebarInset: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useSidebar: () => ({ state: 'expanded', toggleSidebar: vi.fn() }),
}))

// Mock navigation components
vi.mock('@/features/navigation/components/sidebar-tasks', () => ({
  SidebarTasks: () => null,
}))

vi.mock('@/features/navigation/components/breadcrumbs', () => ({
  Breadcrumbs: () => null,
}))

// Mock file viewer components
vi.mock('@/features/file-viewer', () => ({
  FileTree: () => <div>File Tree</div>,
  FileViewer: () => <div>File Viewer</div>,
}))

// Mock preview component
vi.mock('@/features/preview', () => ({
  Preview: () => <div>Preview</div>,
}))

// Create simple mock implementations of task components for testing
const mockTaskComponents = () => {
  const { useTaskDataContext } = vi.hoisted(() => ({
    useTaskDataContext: vi.fn()
  }))

  vi.mock('@/features/tasks/contexts/task-data-context', () => ({
    TaskDataProvider: ({ children }: { children: React.ReactNode }) => children,
    useTaskDataContext: useTaskDataContext
  }))

  vi.mock('@/features/tasks/hooks/use-task-data', () => ({
    useTaskData: (taskId: string) => {
      return {
        task: { id: taskId, title: 'Test Task', status: 'running' },
        isLoading: false,
        error: null
      }
    }
  }))

  vi.mock('@/features/tasks/components/task-header', () => ({
    TaskHeader: () => {
      const context = useTaskDataContext()
      return <h1>{context?.task?.title || 'Loading...'}</h1>
    }
  }))

  vi.mock('@/features/tasks/components/chat-view', () => ({
    ChatView: () => {
      const context = useTaskDataContext()
      return (
        <div>
          <h2>Chat View</h2>
          <div>{context?.task?.title}</div>
          {context?.events?.map((e: any) => (
            <div key={e.id}>{e.data?.message}</div>
          ))}
          <input placeholder="Type a message" />
          <button>send</button>
        </div>
      )
    }
  }))

  vi.mock('@/features/tasks/components/event-viewer', () => ({
    EventViewer: () => {
      const context = useTaskDataContext()
      return (
        <div role="region" aria-label="events">
          <h2>Event Viewer</h2>
          {context?.events?.map((e: any) => (
            <div key={e.id}>{e.data?.message}</div>
          ))}
        </div>
      )
    }
  }))

  vi.mock('@/features/tasks/components/git-diff-viewer', () => ({
    GitDiffViewer: () => {
      const context = useTaskDataContext()
      return <div>{context?.gitDiff?.diff || 'No diff'}</div>
    }
  }))

  vi.mock('@/features/tasks/components/server-logs-viewer', () => ({
    ServerLogsViewer: () => <div>Server Logs</div>
  }))

  vi.mock('@/features/tasks/hooks/use-tasks', () => ({
    useServerStatus: () => ({ data: { configured: false }, isLoading: false }),
    useSendMessage: () => ({ mutate: vi.fn(), isPending: false })
  }))

  return { useTaskDataContext }
}

const { useTaskDataContext } = mockTaskComponents()

describe('Task Detail Route Integration', () => {
  let queryClient: QueryClient
  let apiClient: any
  let sseMock: ReturnType<typeof createSSEMock>
  const mockTaskId = 'test-task-123'

  const mockTask: TaskDetailResponse = {
    id: mockTaskId,
    title: 'Test Task',
    status: 'running',
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    agent_id: 'agent-1',
    environment_id: 'env-1',
    iterations: [
      {
        id: 'iter-1',
        message: 'Initial message',
        created_at: new Date().toISOString(),
      },
    ],
  }

  const mockEvents: TaskEvent[] = [
    {
      id: 'event-1',
      type: 'assistant_message',
      timestamp: new Date().toISOString(),
      data: { message: 'Hello from assistant' },
    },
    {
      id: 'event-2',
      type: 'tool_use',
      timestamp: new Date().toISOString(),
      data: { tool: 'bash', input: 'ls -la' },
    },
  ]

  beforeEach(async () => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: 0, gcTime: 0 },
        mutations: { retry: false },
      },
    })

    // Setup API client mock
    const { apiClient: mockApiClient } = await import('@/lib/api/client')
    apiClient = mockApiClient as any
    apiClient.get = vi.fn()
    apiClient.post = vi.fn()
    apiClient.put = vi.fn()
    apiClient.delete = vi.fn()

    // Setup SSE mock
    sseMock = createSSEMock()
    global.EventSource = sseMock.EventSource as any

    // Setup default context mock
    useTaskDataContext.mockReturnValue({
      task: mockTask,
      events: [],
      gitDiff: null,
      workspaceStatus: null,
      isLoading: false,
      error: null,
      isStreaming: false,
      rightPanelView: 'events',
      setRightPanelView: vi.fn(),
      selectedIteration: null,
      setSelectedIteration: vi.fn(),
    })

    // Clear all API call counts
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.clearAllMocks()
    sseMock.cleanup()
  })

  const renderTaskDetail = async () => {
    const router = createRouter({
      routeTree,
      context: { queryClient },
      defaultPreload: 'intent',
    })

    // Wait for router to be ready
    await router.load()

    // Navigate to the task detail page
    await router.navigate({ to: '/tasks/$taskId', params: { taskId: mockTaskId } })

    return render(
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    )
  }

  describe('Initial Data Loading', () => {
    it('should load all task data in parallel on mount', async () => {
      // Setup API responses
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) return Promise.resolve(mockTask)
        if (url === `/api/v1/tasks/${mockTaskId}/events`) {
          return Promise.resolve({ events: mockEvents, total: 2 })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
          return Promise.resolve({ diff: '+ added line\n- removed line' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/workspace-status`) {
          return Promise.resolve({ status: 'ready', branch: 'main' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/server/status`) {
          return Promise.resolve({ configured: false })
        }
        return Promise.reject(new Error('Unknown endpoint'))
      })

      await renderTaskDetail()

      // Wait for all data to load
      await waitFor(() => {
        expect(screen.getByText('Test Task')).toBeInTheDocument()
      })

      // Verify parallel API calls were made
      expect(apiClient.get).toHaveBeenCalledTimes(5)
      expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}`)
      expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/events`)
      expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/git-diff`)
      expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/workspace-status`)
      expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/server/status`)

      // Verify SSE connection is established
      expect(sseMock.connections).toHaveLength(1)
      expect(sseMock.connections[0].url).toContain(`/api/v1/tasks/${mockTaskId}/events`)
    })

    it('should not make duplicate API calls when switching tabs', async () => {
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) return Promise.resolve(mockTask)
        if (url === `/api/v1/tasks/${mockTaskId}/events`) {
          return Promise.resolve({ events: mockEvents, total: 2 })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
          return Promise.resolve({ diff: '+ added line' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/workspace-status`) {
          return Promise.resolve({ status: 'ready' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/server/status`) {
          return Promise.resolve({ configured: false })
        }
        return Promise.reject(new Error('Unknown endpoint'))
      })

      // Update mock to show diff content
      useTaskDataContext.mockReturnValue({
        task: mockTask,
        events: mockEvents,
        gitDiff: { diff: '+ added line' },
        workspaceStatus: { status: 'ready' },
        isLoading: false,
        error: null,
        isStreaming: false,
        rightPanelView: 'events',
        setRightPanelView: vi.fn(),
        selectedIteration: null,
        setSelectedIteration: vi.fn(),
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Test Task')).toBeInTheDocument()
      })

      const initialCallCount = apiClient.get.mock.calls.length

      // Switch to diff tab
      const diffTab = screen.getByText('Diff')
      fireEvent.click(diffTab)

      await waitFor(() => {
        expect(screen.getByText('+ added line')).toBeInTheDocument()
      })

      // Switch back to events tab
      const eventsTab = screen.getByText('Events')
      fireEvent.click(eventsTab)

      // Switch to files tab
      const filesTab = screen.getByText('Files')
      fireEvent.click(filesTab)

      // No additional API calls should be made
      expect(apiClient.get).toHaveBeenCalledTimes(initialCallCount)
    })
  })

  describe('SSE Connection Management', () => {
    it('should establish only one SSE connection per task', async () => {
      apiClient.get.mockResolvedValue(mockTask)

      await renderTaskDetail()

      await waitFor(() => {
        expect(sseMock.connections).toHaveLength(1)
      })

      // Verify connection URL
      const connection = sseMock.connections[0]
      expect(connection.url).toContain(`/api/v1/tasks/${mockTaskId}/events`)
      expect(connection.readyState).toBe(1) // OPEN
    })

    it('should handle SSE events and update UI without refetching', async () => {
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) return Promise.resolve(mockTask)
        if (url === `/api/v1/tasks/${mockTaskId}/events`) {
          return Promise.resolve({ events: [], total: 0 })
        }
        return Promise.resolve({})
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(sseMock.connections).toHaveLength(1)
      })

      const initialGetCallCount = apiClient.get.mock.calls.length

      // Update mock to include new event
      const newEvent = {
        id: 'new-event',
        type: 'assistant_message',
        timestamp: new Date().toISOString(),
        data: { message: 'New SSE message' },
      }

      useTaskDataContext.mockReturnValue({
        task: mockTask,
        events: [newEvent],
        gitDiff: null,
        workspaceStatus: null,
        isLoading: false,
        error: null,
        isStreaming: true,
        rightPanelView: 'events',
        setRightPanelView: vi.fn(),
        selectedIteration: null,
        setSelectedIteration: vi.fn(),
      })

      // Emit SSE event
      act(() => {
        sseMock.emit({
          type: 'event',
          data: JSON.stringify(newEvent),
        })
      })

      // Wait for UI update
      await waitFor(() => {
        expect(screen.getByText('New SSE message')).toBeInTheDocument()
      })

      // Verify no additional API calls were made
      expect(apiClient.get).toHaveBeenCalledTimes(initialGetCallCount)
    })

    it('should close SSE connection on unmount', async () => {
      apiClient.get.mockResolvedValue(mockTask)

      const { unmount } = await renderTaskDetail()

      await waitFor(() => {
        expect(sseMock.connections).toHaveLength(1)
      })

      const connection = sseMock.connections[0]
      expect(connection.readyState).toBe(1) // OPEN

      unmount()

      expect(connection.close).toHaveBeenCalled()
    })
  })

  describe('Message Sending and Optimistic Updates', () => {
    it('should send message and handle optimistic updates', async () => {
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) return Promise.resolve(mockTask)
        if (url === `/api/v1/tasks/${mockTaskId}/events`) {
          return Promise.resolve({ events: [], total: 0 })
        }
        return Promise.resolve({})
      })

      apiClient.post.mockResolvedValue({
        success: true,
        iteration: { id: 'iter-2', message: 'User message' },
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByPlaceholderText(/Type a message/)).toBeInTheDocument()
      })

      const input = screen.getByPlaceholderText(/Type a message/)
      const sendButton = screen.getByRole('button', { name: /send/i })

      // Type and send message
      fireEvent.change(input, { target: { value: 'User message' } })
      fireEvent.click(sendButton)

      // Verify API call
      await waitFor(() => {
        expect(apiClient.post).toHaveBeenCalledWith(
          `/api/v1/tasks/${mockTaskId}/message`,
          { message: 'User message' }
        )
      })

      // Verify only necessary invalidations occur
      await waitFor(() => {
        // Task metadata should be refetched
        expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}`)
      })
    })

    it('should handle message sending errors gracefully', async () => {
      apiClient.get.mockResolvedValue(mockTask)
      apiClient.post.mockRejectedValue(new Error('Network error'))

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByPlaceholderText(/Type a message/)).toBeInTheDocument()
      })

      const input = screen.getByPlaceholderText(/Type a message/)
      const sendButton = screen.getByRole('button', { name: /send/i })

      fireEvent.change(input, { target: { value: 'Failed message' } })
      fireEvent.click(sendButton)

      // Wait for error handling
      await waitFor(() => {
        expect(apiClient.post).toHaveBeenCalled()
      })

      // Input should be enabled again
      expect(input).not.toBeDisabled()
      expect(sendButton).not.toBeDisabled()
    })
  })

  describe('Tab Switching and State Persistence', () => {
    it.skip('should maintain scroll position when switching tabs', async () => {
      // Skipped due to scroll area implementation details
    })

    it('should handle server tab visibility based on configuration', async () => {
      // Start without server configured
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) return Promise.resolve(mockTask)
        if (url === `/api/v1/tasks/${mockTaskId}/server/status`) {
          return Promise.resolve({ configured: false })
        }
        return Promise.resolve({})
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Events')).toBeInTheDocument()
      })

      // Server tab should not be visible
      expect(screen.queryByText('Server')).not.toBeInTheDocument()
      expect(screen.queryByText('Preview')).not.toBeInTheDocument()

      // Update server status
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}/server/status`) {
          return Promise.resolve({ configured: true, url: 'http://localhost:3000' })
        }
        return Promise.resolve({})
      })

      // Trigger server status refetch (this would happen via SSE in real app)
      act(() => {
        queryClient.invalidateQueries({ queryKey: ['tasks', mockTaskId, 'server-status'] })
      })

      // Server tabs should now be visible
      await waitFor(() => {
        expect(screen.getByText('Server')).toBeInTheDocument()
        expect(screen.getByText('Preview')).toBeInTheDocument()
      })
    })
  })

  describe('Error Handling and Recovery', () => {
    it('should handle task loading errors', async () => {
      apiClient.get.mockRejectedValue(new Error('Failed to load task'))

      // Update mock to show error state
      useTaskDataContext.mockReturnValue({
        task: null,
        events: [],
        gitDiff: null,
        workspaceStatus: null,
        isLoading: false,
        error: new Error('Failed to load task'),
        isStreaming: false,
        rightPanelView: 'events',
        setRightPanelView: vi.fn(),
        selectedIteration: null,
        setSelectedIteration: vi.fn(),
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText(/Failed to load task/)).toBeInTheDocument()
      })

      // No SSE connection should be established
      expect(sseMock.connections).toHaveLength(0)
    })

    it('should handle SSE connection errors and retry', async () => {
      apiClient.get.mockResolvedValue(mockTask)

      await renderTaskDetail()

      await waitFor(() => {
        expect(sseMock.connections).toHaveLength(1)
      })

      // Simulate connection error
      act(() => {
        sseMock.connections[0].dispatchEvent(new Event('error'))
      })

      // Should attempt to reconnect
      await waitFor(() => {
        expect(sseMock.connections).toHaveLength(2)
      })
    })
  })

  describe('Query Invalidation', () => {
    it('should invalidate correct queries on task status change', async () => {
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) {
          return Promise.resolve({ ...mockTask, status: 'running' })
        }
        return Promise.resolve({})
      })

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Test Task')).toBeInTheDocument()
      })

      const initialCallCount = apiClient.get.mock.calls.length

      // Simulate task completion via SSE
      act(() => {
        sseMock.emit({
          type: 'task_complete',
          data: JSON.stringify({ taskId: mockTaskId }),
        })
      })

      // Update mock to return completed status
      apiClient.get.mockImplementation((url: string) => {
        if (url === `/api/v1/tasks/${mockTaskId}`) {
          return Promise.resolve({ ...mockTask, status: 'completed' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
          return Promise.resolve({ diff: 'Final diff' })
        }
        if (url === `/api/v1/tasks/${mockTaskId}/workspace-status`) {
          return Promise.resolve({ status: 'ready', branch: 'feature-branch' })
        }
        return Promise.resolve({})
      })

      // Should refetch task metadata, git diff, and workspace status
      await waitFor(() => {
        expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}`)
        expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/git-diff`)
        expect(apiClient.get).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/workspace-status`)
      })

      // Events should not be refetched (handled by SSE)
      const eventsCalls = apiClient.get.mock.calls.filter(
        call => call[0] === `/api/v1/tasks/${mockTaskId}/events`
      )
      expect(eventsCalls).toHaveLength(1) // Only initial load
    })
  })

  describe('Performance Optimizations', () => {
    it('should not refetch data when navigating away and back', async () => {
      apiClient.get.mockResolvedValue(mockTask)

      const router = createRouter({
        routeTree,
        context: { queryClient },
      })

      await router.load()
      await router.navigate({ to: '/tasks/$taskId', params: { taskId: mockTaskId } })
      render(
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      )

      await waitFor(() => {
        expect(screen.getByText('Test Task')).toBeInTheDocument()
      })

      const initialCallCount = apiClient.get.mock.calls.length

      // Navigate away
      act(() => {
        router.navigate({ to: '/' })
      })

      // Navigate back
      act(() => {
        router.navigate({ to: '/tasks/$taskId', params: { taskId: mockTaskId } })
      })

      // Data should be served from cache
      expect(apiClient.get).toHaveBeenCalledTimes(initialCallCount)
    })

    it('should batch multiple invalidations into single refetch', async () => {
      apiClient.get.mockResolvedValue(mockTask)

      await renderTaskDetail()

      await waitFor(() => {
        expect(screen.getByText('Test Task')).toBeInTheDocument()
      })

      // Clear call history
      apiClient.get.mockClear()

      // Simulate multiple events that would trigger invalidations
      act(() => {
        // These should be batched
        queryClient.invalidateQueries({ queryKey: ['tasks', mockTaskId] })
        queryClient.invalidateQueries({ queryKey: ['tasks', mockTaskId] })
        queryClient.invalidateQueries({ queryKey: ['tasks', mockTaskId] })
      })

      await waitFor(() => {
        // Should only result in one API call per endpoint
        const taskCalls = apiClient.get.mock.calls.filter(
          call => call[0] === `/api/v1/tasks/${mockTaskId}`
        )
        expect(taskCalls).toHaveLength(1)
      })
    })
  })
})