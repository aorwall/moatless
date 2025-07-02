import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'
import {
  createMockAgentTriggerResponse,
  createMockTask,
  createMockTaskWithIterations,
} from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'

import { useSendMessage, useTask } from '../use-tasks'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    sendMessage: vi.fn(),
    getTask: vi.fn(),
  },
}))

describe('useSendMessage Integration Test', () => {
  it('should refetch task data after sending a message', async () => {
    const taskId = 'task-123'

    // Initial task state with 1 iteration
    const initialTask = createMockTaskWithIterations(
      {
        task_id: taskId,
        title: 'Test Task',
        num_turns: 1,
        total_cost_usd: 0.01,
        status: 'completed',
        is_running: false,
      },
      1,
    )

    // Updated task state with 2 iterations after message sent
    const updatedTask = createMockTaskWithIterations(
      {
        task_id: taskId,
        title: 'Test Task',
        num_turns: 2,
        total_cost_usd: 0.02,
        status: 'active',
        is_running: true,
      },
      2,
    )

    // Mock initial task fetch
    vi.mocked(tasksApi.getTask).mockResolvedValueOnce(initialTask)

    const wrapper = createWrapper()

    // First, render the useTask hook to establish initial state
    const { result: taskResult, rerender: rerenderTask } = renderHook(() => useTask(taskId), {
      wrapper,
    })

    // Wait for initial data to load
    await waitFor(() => {
      expect(taskResult.current.isSuccess).toBe(true)
      expect(taskResult.current.data).toEqual(initialTask)
    })

    // Mock the send message response
    vi.mocked(tasksApi.sendMessage).mockResolvedValueOnce(
      createMockAgentTriggerResponse({ taskId: taskId }),
    )

    // Mock the refetch to return updated data
    vi.mocked(tasksApi.getTask).mockResolvedValueOnce(updatedTask)

    // Now render the useSendMessage hook
    const { result: sendMessageResult } = renderHook(() => useSendMessage(), {
      wrapper,
    })

    // Send a message
    await act(async () => {
      await sendMessageResult.current.mutateAsync({
        taskId,
        message: 'Test message',
      })
    })

    // Wait for the task data to be refetched
    await waitFor(() => {
      // Re-render to pick up the updated cache
      rerenderTask()

      // Check that the task data has been updated
      expect(taskResult.current.data).toEqual(updatedTask)
      expect(taskResult.current.data?.iterations).toHaveLength(2)
      expect(taskResult.current.data?.is_running).toBe(true)
      expect(taskResult.current.data?.status).toBe('active')
    })

    // Verify API calls
    expect(tasksApi.sendMessage).toHaveBeenCalledWith(taskId, 'Test message', undefined, undefined, undefined, undefined, undefined)
    // In event-driven architecture, explicit refetch calls are reduced
    // Initial call + immediate refetch after message
    expect(tasksApi.getTask).toHaveBeenCalledTimes(2)
  })

  it('should handle concurrent message sends correctly', async () => {
    const taskId = 'task-123'
    const mockTask = createMockTask({ task_id: taskId })

    vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)
    vi.mocked(tasksApi.sendMessage).mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(() => resolve(createMockAgentTriggerResponse()), 100)
        }),
    )

    const wrapper = createWrapper()
    const { result } = renderHook(() => useSendMessage(), { wrapper })

    // Send two messages concurrently
    const promise1 = result.current.mutateAsync({
      taskId,
      message: 'First message',
    })

    const promise2 = result.current.mutateAsync({
      taskId,
      message: 'Second message',
    })

    // Both should complete without errors
    await expect(Promise.all([promise1, promise2])).resolves.toBeDefined()

    // Both messages should have been sent
    expect(tasksApi.sendMessage).toHaveBeenCalledWith(taskId, 'First message', undefined, undefined, undefined, undefined, undefined)
    expect(tasksApi.sendMessage).toHaveBeenCalledWith(taskId, 'Second message', undefined, undefined, undefined, undefined, undefined)
  })

  it('should properly handle errors and not invalidate on failure', async () => {
    const taskId = 'task-123'
    const mockTask = createMockTask({ task_id: taskId })
    const error = new Error('Network error')

    vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)
    vi.mocked(tasksApi.sendMessage).mockRejectedValueOnce(error)

    const wrapper = createWrapper()
    const spyInvalidate = vi.spyOn(wrapper({ children: null }).props.client, 'invalidateQueries')

    const { result } = renderHook(() => useSendMessage(), { wrapper })

    // Try to send a message that will fail
    await expect(
      result.current.mutateAsync({
        taskId,
        message: 'This will fail',
      }),
    ).rejects.toThrow('Network error')

    // Verify that queries were NOT invalidated on failure
    expect(spyInvalidate).not.toHaveBeenCalled()
  })

  it('should maintain correct query key hierarchy in event-driven architecture', async () => {
    const taskId = 'task-123'

    vi.mocked(tasksApi.sendMessage).mockResolvedValue(createMockAgentTriggerResponse())

    const wrapper = createWrapper()
    const testWrapper = wrapper({ children: null })
    const client = testWrapper.props.client
    const spyInvalidate = vi.spyOn(client, 'invalidateQueries')
    const spyRefetch = vi.spyOn(client, 'refetchQueries')

    const { result } = renderHook(() => useSendMessage(), { wrapper })

    await act(async () => {
      await result.current.mutateAsync({
        taskId,
        message: 'Test hierarchy',
      })
    })

    // In the new event-driven architecture, mutations only invalidate task lists
    // Task-specific queries are invalidated by events, not mutations
    expect(spyInvalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.tasksList(),
    })
    
    // Task-specific queries should NOT be invalidated by mutations
    expect(spyInvalidate).not.toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })
    expect(spyInvalidate).not.toHaveBeenCalledWith({
      queryKey: queryKeys.taskWorkspaceStatus(taskId),
    })

    // Verify immediate refetch of task data still happens
    expect(spyRefetch).toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })
  })
})
