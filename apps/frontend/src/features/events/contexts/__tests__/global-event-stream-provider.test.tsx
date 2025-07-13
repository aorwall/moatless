import { EventType } from '@moatless/types/types/event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import React from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import { GlobalEventStreamProvider, useGlobalEventStream } from '../global-event-stream-provider'

// Mock the API client
vi.mock('@/lib/api/client', () => ({
  api: {
    events: {
      stream: {
        $get: vi.fn(),
      },
    },
  },
}))

// Test component to access the hook
function TestComponent({ onEvent }: { onEvent: (event: any) => void }) {
  const { lastEvent, subscribeToTask, subscribeToType } = useGlobalEventStream()

  // Subscribe to test events
  React.useEffect(() => {
    const unsubscribeTask = subscribeToTask('task-123', onEvent)
    const unsubscribeType = subscribeToType(EventType.TASK_STATUS, onEvent)

    return () => {
      unsubscribeTask()
      unsubscribeType()
    }
  }, [subscribeToTask, subscribeToType, onEvent])

  return <div>{lastEvent ? `Last event: ${lastEvent.type}` : 'No events'}</div>
}

describe('GlobalEventStreamProvider', () => {
  let queryClient: QueryClient
  let mockEventSource: any
  let eventListeners: Record<string, any[]> = {}

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })

    // Mock EventSource
    eventListeners = {}
    mockEventSource = {
      addEventListener: vi.fn((event: string, handler: any) => {
        if (!eventListeners[event]) {
          eventListeners[event] = []
        }
        eventListeners[event].push(handler)
      }),
      removeEventListener: vi.fn(),
      close: vi.fn(),
      readyState: 1, // OPEN
    }

    global.EventSource = vi.fn(() => mockEventSource) as any

    // Spy on query invalidation
    vi.spyOn(queryClient, 'invalidateQueries')
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  const renderWithProviders = (children: React.ReactNode) => {
    return render(
      <QueryClientProvider client={queryClient}>
        <GlobalEventStreamProvider>{children}</GlobalEventStreamProvider>
      </QueryClientProvider>,
    )
  }

  const simulateEvent = (event: any) => {
    const handlers = eventListeners.message || []
    handlers.forEach((handler) => {
      handler({ data: JSON.stringify(event) })
    })
  }

  it('should invalidate only specific task on TASK_STATUS event', async () => {
    const onEvent = vi.fn()

    renderWithProviders(<TestComponent onEvent={onEvent} />)

    // Wait for SSE connection to be established
    await waitFor(() => {
      expect(global.EventSource).toHaveBeenCalled()
    })

    // Simulate a task status event
    const taskStatusEvent = {
      id: 'event-1',
      type: EventType.TASK_STATUS,
      taskId: 'task-123',
      timestamp: Date.now(),
      data: { status: 'running', previousStatus: 'created' },
    }

    simulateEvent(taskStatusEvent)

    await waitFor(() => {
      // Should invalidate the specific task detail
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.task('task-123'),
      })

      // Should invalidate task lists (for sidebar update)
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.lists(),
      })

      // Should NOT invalidate all tasks
      expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.all,
      })
    })
  })

  it('should handle multiple task events without cross-task invalidation', async () => {
    const onEvent = vi.fn()

    renderWithProviders(<TestComponent onEvent={onEvent} />)

    await waitFor(() => {
      expect(global.EventSource).toHaveBeenCalled()
    })

    // Simulate events for different tasks
    const task1Event = {
      id: 'event-1',
      type: EventType.TASK_STATUS,
      taskId: 'task-123',
      timestamp: Date.now(),
      data: { status: 'running' },
    }

    const task2Event = {
      id: 'event-2',
      type: EventType.RESULT,
      taskId: 'task-456',
      timestamp: Date.now(),
      data: { result: 'completed' },
    }

    simulateEvent(task1Event)
    simulateEvent(task2Event)

    await waitFor(() => {
      // Task 1 invalidations
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.task('task-123'),
      })

      // Task 2 invalidations
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.task('task-456'),
      })
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.taskGitDiff('task-456'),
      })

      // Should not invalidate task-123 when task-456 has an event
      const task123Calls = (queryClient.invalidateQueries as any).mock.calls.filter((call: any) =>
        JSON.stringify(call[0].queryKey).includes('task-123'),
      )
      const task456Calls = (queryClient.invalidateQueries as any).mock.calls.filter((call: any) =>
        JSON.stringify(call[0].queryKey).includes('task-456'),
      )

      // Each task should only have its own invalidations
      expect(task123Calls.length).toBe(1) // Only from task-123 event
      expect(task456Calls.length).toBe(2) // From task-456 event (task + gitDiff)
    })
  })

  it('should handle legacy subType invalidations correctly', async () => {
    const onEvent = vi.fn()

    renderWithProviders(<TestComponent onEvent={onEvent} />)

    await waitFor(() => {
      expect(global.EventSource).toHaveBeenCalled()
    })

    // Simulate a legacy event with subType
    const legacyEvent = {
      id: 'event-1',
      type: EventType.MESSAGE,
      taskId: 'task-789',
      subType: 'task:statusChanged',
      timestamp: Date.now(),
      data: {},
    }

    simulateEvent(legacyEvent)

    await waitFor(() => {
      // Should invalidate task detail
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.detail('task-789'),
      })

      // Should invalidate task lists
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.lists(),
      })

      // Should NOT use the broad tasks.all invalidation
      expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.all,
      })
    })
  })

  it('should only invalidate task lists for status-affecting events', async () => {
    const onEvent = vi.fn()

    renderWithProviders(<TestComponent onEvent={onEvent} />)

    await waitFor(() => {
      expect(global.EventSource).toHaveBeenCalled()
    })

    // Non-status-affecting event
    const workspaceEvent = {
      id: 'event-1',
      type: EventType.WORKSPACE,
      taskId: 'task-111',
      timestamp: Date.now(),
      data: {},
    }

    simulateEvent(workspaceEvent)

    await waitFor(() => {
      // Should invalidate workspace-specific queries
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.taskWorkspaceStatus('task-111'),
      })
      expect(queryClient.invalidateQueries).toHaveBeenCalledWith({
        queryKey: queryKeys.taskGitDiff('task-111'),
      })

      // Should NOT invalidate task lists
      expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.lists(),
      })
    })
  })
})
