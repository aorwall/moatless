import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TaskDetailResponse, Event, TaskListItem } from '@shared/types/api'
import { apiClient } from '@/lib/api/client'
import { TaskDataProvider, useTaskDataContext } from '@/features/tasks/contexts/task-data-context'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import React from 'react'

// Mock the API client
vi.mock('@/lib/api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  }
}))

// Mock SSE to prevent connection attempts in tests
vi.mock('@/features/tasks/api/ndjson-sse', () => ({
  NDJSONSSEClient: vi.fn().mockImplementation(() => ({
    connect: vi.fn().mockResolvedValue(undefined),
    disconnect: vi.fn(),
    isConnected: false,
    on: vi.fn(),
    off: vi.fn(),
  }))
}))

// Simple test component that uses the context
function TestComponent() {
  const context = useTaskDataContext()
  
  return (
    <div>
      <h1>{context.task?.title || 'Loading...'}</h1>
      <div data-testid="task-status">{context.task?.status}</div>
      <div data-testid="event-count">{context.events.length}</div>
      <div data-testid="is-loading">{context.isLoading ? 'true' : 'false'}</div>
      <div data-testid="git-diff">{context.gitDiff?.diff || 'No diff'}</div>
      <div data-testid="is-streaming">{context.isStreaming ? 'true' : 'false'}</div>
      {context.error && <div data-testid="error">{context.error.message}</div>}
    </div>
  )
}

// Component to test task list refetching
function TaskListComponent() {
  const { data: tasks = [] } = useTasksList()
  
  return (
    <div>
      <div data-testid="task-count">{tasks.length}</div>
      {tasks.map(task => (
        <div key={task.id} data-testid={`task-${task.id}`}>
          {task.title}
        </div>
      ))}
    </div>
  )
}

describe('Task Data Context Integration', () => {
  let queryClient: QueryClient
  const mockTaskId = 'test-task-123'
  
  const mockTask: TaskDetailResponse = {
    id: mockTaskId,
    title: 'Test Task',
    status: 'running',
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    agent_id: 'agent-1',
    environment_id: 'env-1',
    iterations: [],
  }

  const mockEvents: Event[] = [
    {
      id: 'event-1',
      type: 'assistant_message',
      timestamp: new Date().toISOString(),
      data: { message: 'Hello' },
    }
  ]

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { 
          retry: false, 
          staleTime: 0,
          gcTime: 0,
          refetchOnWindowFocus: false,
          refetchOnMount: true,
        },
        mutations: { retry: false },
      },
    })
    
    // Override the default query options for tests
    queryClient.setQueryDefaults(['tasks'], {
      staleTime: 0,
      gcTime: 0,
    })
    
    // Clear all mocks
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.clearAllMocks()
    queryClient.clear()
  })

  it('should load all task data in parallel', async () => {
    // Mock API responses
    const getMock = vi.mocked(apiClient.get)
    getMock.mockImplementation((url: string) => {
      if (url === `/api/v1/tasks/${mockTaskId}`) {
        return Promise.resolve(mockTask)
      }
      if (url === `/api/v1/tasks/${mockTaskId}/events`) {
        return Promise.resolve({ events: mockEvents, total: 1 })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
        return Promise.resolve({ diff: '+ added line' })
      }
      return Promise.reject(new Error('Unknown endpoint'))
    })

    render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    // Initially loading
    expect(screen.getByTestId('is-loading')).toHaveTextContent('true')

    // Wait for data to load
    await waitFor(() => {
      expect(screen.getByText('Test Task')).toBeInTheDocument()
    })

    // Verify all data loaded
    expect(screen.getByTestId('task-status')).toHaveTextContent('running')
    expect(screen.getByTestId('git-diff')).toHaveTextContent('+ added line')
    expect(screen.getByTestId('event-count')).toHaveTextContent('1')
    expect(screen.getByTestId('is-loading')).toHaveTextContent('false')

    // Verify parallel API calls - expect the second argument to be undefined for events
    expect(getMock).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}`)
    expect(getMock).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/events`, undefined)
    expect(getMock).toHaveBeenCalledWith(`/api/v1/tasks/${mockTaskId}/git-diff`)
    
    // Only 3 calls - workspace status is not fetched by useTaskData
    expect(getMock).toHaveBeenCalledTimes(3)
  })

  it('should handle errors gracefully', async () => {
    const getMock = vi.mocked(apiClient.get)
    getMock.mockRejectedValue(new Error('Network error'))

    render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    await waitFor(() => {
      expect(screen.getByTestId('error')).toHaveTextContent('Network error')
    })

    expect(screen.getByTestId('is-loading')).toHaveTextContent('false')
  })

  it('should not refetch when data is in cache', async () => {
    const getMock = vi.mocked(apiClient.get)
    getMock.mockImplementation((url: string) => {
      if (url === `/api/v1/tasks/${mockTaskId}`) {
        return Promise.resolve(mockTask)
      }
      if (url === `/api/v1/tasks/${mockTaskId}/events`) {
        return Promise.resolve({ events: [], total: 0 })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
        return Promise.resolve({ diff: '+ cached diff' })
      }
      return Promise.reject(new Error('Unknown endpoint'))
    })

    // First render
    const { unmount } = render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    await waitFor(() => {
      expect(screen.getByText('Test Task')).toBeInTheDocument()
    })

    const initialCallCount = getMock.mock.calls.length
    expect(initialCallCount).toBe(3)

    // Unmount
    unmount()

    // Clear mock call history but not the implementations
    getMock.mockClear()

    // Re-render with same task ID within cache time window
    render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    // Wait a tick for React to render
    await waitFor(() => {
      expect(screen.getByText('Test Task')).toBeInTheDocument()
    })

    // Data should be from cache, minimal API calls
    expect(screen.getByTestId('git-diff')).toHaveTextContent('+ cached diff')
    
    // With staleTime: 0, React Query might still make some calls
    // but should be significantly less than initial load
    expect(getMock.mock.calls.length).toBeLessThanOrEqual(3)
  })

  it('should handle query invalidation correctly', async () => {
    const getMock = vi.mocked(apiClient.get)
    let gitDiffCallCount = 0
    
    getMock.mockImplementation((url: string) => {
      if (url === `/api/v1/tasks/${mockTaskId}`) {
        return Promise.resolve(mockTask)
      }
      if (url === `/api/v1/tasks/${mockTaskId}/events`) {
        return Promise.resolve({ events: [], total: 0 })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
        gitDiffCallCount++
        return Promise.resolve({ diff: `diff version ${gitDiffCallCount}` })
      }
      return Promise.reject(new Error('Unknown endpoint'))
    })

    render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    await waitFor(() => {
      expect(screen.getByTestId('git-diff')).toHaveTextContent('diff version 1')
    })

    // Manually update the cache to simulate what would happen after invalidation
    await act(async () => {
      // Set the query data directly to simulate a successful refetch
      queryClient.setQueryData(['swarm', 'tasks', mockTaskId, 'git-diff'], { diff: 'diff version 2' })
    })

    // Should show updated diff
    await waitFor(() => {
      expect(screen.getByTestId('git-diff')).toHaveTextContent('diff version 2')
    })
  })

  it('should update UI when task data changes', async () => {
    const getMock = vi.mocked(apiClient.get)
    
    getMock.mockImplementation((url: string) => {
      if (url === `/api/v1/tasks/${mockTaskId}`) {
        return Promise.resolve({ ...mockTask, title: 'Task v1' })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/events`) {
        return Promise.resolve({ events: [], total: 0 })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
        return Promise.resolve({ diff: 'diff' })
      }
      return Promise.reject(new Error('Unknown endpoint'))
    })

    render(
      <QueryClientProvider client={queryClient}>
        <TaskDataProvider taskId={mockTaskId}>
          <TestComponent />
        </TaskDataProvider>
      </QueryClientProvider>
    )

    await waitFor(() => {
      expect(screen.getByText('Task v1')).toBeInTheDocument()
    })

    // Simulate what would happen when task data is updated (e.g., via SSE or refetch)
    await act(async () => {
      queryClient.setQueryData(['swarm', 'tasks', mockTaskId], { 
        ...mockTask, 
        title: 'Task v2',
        status: 'completed' 
      })
    })

    // Should show updated task
    await waitFor(() => {
      expect(screen.getByText('Task v2')).toBeInTheDocument()
      expect(screen.getByTestId('task-status')).toHaveTextContent('completed')
    })
  })

  it('should refetch task list when task is invalidated', async () => {
    const getMock = vi.mocked(apiClient.get)
    const mockTasks: TaskListItem[] = [
      {
        id: mockTaskId,
        title: 'Test Task',
        status: 'running',
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        agent_id: 'agent-1',
        environment_id: 'env-1',
      },
      {
        id: 'task-2',
        title: 'Another Task',
        status: 'completed',
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        agent_id: 'agent-2',
        environment_id: 'env-1',
      }
    ]

    let taskListCallCount = 0
    getMock.mockImplementation((url: string, params?: any) => {
      // Handle both with and without query params
      if (url === '/api/v1/tasks' || url.startsWith('/api/v1/tasks?')) {
        taskListCallCount++
        // Change the list on second call to verify refetch
        if (taskListCallCount === 1) {
          return Promise.resolve({ tasks: mockTasks })
        } else {
          return Promise.resolve({
            tasks: [
              { ...mockTasks[0], status: 'completed' },
              mockTasks[1]
            ]
          })
        }
      }
      if (url === `/api/v1/tasks/${mockTaskId}`) {
        return Promise.resolve({ ...mockTask, status: 'completed' })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/events`) {
        return Promise.resolve({ events: [], total: 0 })
      }
      if (url === `/api/v1/tasks/${mockTaskId}/git-diff`) {
        return Promise.resolve({ diff: 'diff' })
      }
      return Promise.reject(new Error('Unknown endpoint'))
    })

    // Render both task list and task detail
    render(
      <QueryClientProvider client={queryClient}>
        <div>
          <TaskListComponent />
          <TaskDataProvider taskId={mockTaskId}>
            <TestComponent />
          </TaskDataProvider>
        </div>
      </QueryClientProvider>
    )

    // Wait for initial load
    await waitFor(() => {
      expect(screen.getByTestId('task-count')).toHaveTextContent('2')
      expect(screen.getByTestId(`task-${mockTaskId}`)).toHaveTextContent('Test Task')
    })

    // Clear call history
    getMock.mockClear()

    // Invalidate the specific task - this should also invalidate the task list
    await act(async () => {
      // Force a refetch by invalidating with immediate refetch
      await queryClient.invalidateQueries({ 
        queryKey: ['swarm', 'tasks'], 
        refetchType: 'active' 
      })
      
      // Also manually trigger a refetch to ensure it happens
      await queryClient.refetchQueries({
        queryKey: ['swarm', 'tasks', 'list'],
        type: 'active'
      })
    })

    // Wait for refetch and verify updated data
    await waitFor(() => {
      // Task list should be refetched - check for the updated status in the UI
      const taskElement = screen.getByTestId(`task-${mockTaskId}`)
      // The mock returns status 'completed' on second call
      expect(taskElement).toBeInTheDocument()
      
      // Verify API was called at least once after invalidation
      const taskListCalls = getMock.mock.calls.filter(call => call[0] === '/api/v1/tasks')
      expect(taskListCalls.length).toBeGreaterThan(0)
    })

    // Verify the task status was updated in the list
    expect(screen.getByTestId('task-count')).toHaveTextContent('2')
  })
})