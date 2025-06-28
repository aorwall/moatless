import { EventType } from '@shared/types/event'
import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NDJSONSSEClient } from '@/features/tasks/api/ndjson-sse'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { createMockEvent, createMockTask, createMockWorkspaceStatus } from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'

import { useTaskStreaming } from '../use-task-streaming'

// Mock the NDJSONSSEClient
vi.mock('@/features/tasks/api/ndjson-sse', () => ({
  NDJSONSSEClient: vi.fn(),
}))

// Mock the tasks API module
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    getTask: vi.fn(),
    getTaskWorkspaceStatus: vi.fn(),
  },
}))

describe('useTaskStreaming', () => {
  const mockTaskId = 'test-task-123'

  // Mock SSE client instance
  const mockConnect = vi.fn()
  const mockDisconnect = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Reset mock SSE client
    vi.mocked(NDJSONSSEClient).mockImplementation(
      (options) =>
        ({
          connect: mockConnect,
          disconnect: mockDisconnect,
          onEvent: options.onEvent,
          onError: options.onError,
          onConnect: options.onConnect,
          onDisconnect: options.onDisconnect,
          onTaskNotFound: options.onTaskNotFound,
        }) as any,
    )

    // Default API mock returns
    vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'open' }))
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue(
      createMockWorkspaceStatus({ status: 'idle' as any }),
    )
  })

  describe('streaming behavior', () => {
    it('should stream when task is running', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(mockConnect).toHaveBeenCalled()
      })

      expect(NDJSONSSEClient).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: mockTaskId,
        }),
      )
    })

    it('should not stream when task is closed', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'closed' }))

      renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      // Wait to ensure no connection is made
      await new Promise((resolve) => setTimeout(resolve, 100))
      expect(mockConnect).not.toHaveBeenCalled()
    })
  })

  describe('event handling without reconnection loops', () => {
    it('should not recreate SSE client when events are received', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const { result } = renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(NDJSONSSEClient).toHaveBeenCalledTimes(1)
        expect(mockConnect).toHaveBeenCalledTimes(1)
      })

      // Get the onEvent callback from the SSE client constructor
      const sseClientCall = vi.mocked(NDJSONSSEClient).mock.calls[0][0]
      const onEvent = sseClientCall.onEvent!

      // Simulate receiving multiple events with different eventIds
      const event1 = createMockEvent({ eventId: 'event-1' })
      const event2 = createMockEvent({ eventId: 'event-2' })
      const event3 = createMockEvent({ eventId: 'event-3' })

      onEvent(event1)
      onEvent(event2)
      onEvent(event3)

      // Wait to ensure no additional SSE clients are created
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Critical test: Should still only have one SSE client instance
      expect(NDJSONSSEClient).toHaveBeenCalledTimes(1)
      expect(mockConnect).toHaveBeenCalledTimes(1)
      expect(mockDisconnect).not.toHaveBeenCalled()

      // All events should be in the stream
      expect(result.current.streamEvents).toHaveLength(3)
      expect(result.current.streamEvents).toContain(event1)
      expect(result.current.streamEvents).toContain(event2)
      expect(result.current.streamEvents).toContain(event3)
    })

    it('should add events to stream when received', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const { result } = renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      // Wait for SSE client to be created
      await waitFor(() => {
        expect(NDJSONSSEClient).toHaveBeenCalled()
      })

      // Get the onEvent callback from the SSE client constructor
      const sseClientCall = vi.mocked(NDJSONSSEClient).mock.calls[0][0]
      const onEvent = sseClientCall.onEvent!

      // Simulate receiving an event
      const mockEvent = createMockEvent({
        type: EventType.MESSAGE,
        eventId: 'event-1',
      })

      onEvent(mockEvent)

      expect(result.current.streamEvents).toContain(mockEvent)
      expect(result.current.streamingError).toBeNull()
    })

    it('should invalidate queries on RESULT events', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const wrapper = createWrapper()
      const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

      renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper,
      })

      await waitFor(() => {
        expect(NDJSONSSEClient).toHaveBeenCalled()
      })

      // Get the onEvent callback
      const sseClientCall = vi.mocked(NDJSONSSEClient).mock.calls[0][0]
      const onEvent = sseClientCall.onEvent!

      // Simulate receiving a RESULT event
      const resultEvent = createMockEvent({
        type: EventType.RESULT,
        eventId: 'result-1',
      })

      onEvent(resultEvent)

      // Should invalidate task queries
      expect(spyInvalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.task(mockTaskId),
      })
    })
  })

  describe('connection lifecycle', () => {
    it('should disconnect SSE client on unmount', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const { unmount } = renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(mockConnect).toHaveBeenCalled()
      })

      unmount()

      expect(mockDisconnect).toHaveBeenCalled()
    })

    it('should set isStreaming to true when connected', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const { result } = renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(NDJSONSSEClient).toHaveBeenCalled()
      })

      // Get the onConnect callback
      const sseClientCall = vi.mocked(NDJSONSSEClient).mock.calls[0][0]
      const onConnect = sseClientCall.onConnect!

      onConnect()

      expect(result.current.isStreaming).toBe(true)
      expect(result.current.streamingError).toBeNull()
    })

    it('should handle SSE errors', async () => {
      vi.mocked(tasksApi.getTask).mockResolvedValue(createMockTask({ status: 'running' }))

      const { result } = renderHook(() => useTaskStreaming(mockTaskId), {
        wrapper: createWrapper(),
      })

      await waitFor(() => {
        expect(NDJSONSSEClient).toHaveBeenCalled()
      })

      // Get the onError callback
      const sseClientCall = vi.mocked(NDJSONSSEClient).mock.calls[0][0]
      const onError = sseClientCall.onError!

      // Simulate an error
      const error = new Error('Connection failed')
      onError(error)

      expect(result.current.streamingError).toBe('Streaming connection error')
    })
  })
})
