import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { tasksApi } from '@/features/tasks/api/tasks'
import { useSendMessage, useTask } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import {
  createMockAgentTriggerResponse,
  createMockTaskWithIterations,
} from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    sendMessage: vi.fn(),
    getTask: vi.fn(),
    getTaskWorkspaceStatus: vi.fn(),
    listTasks: vi.fn(),
  },
}))

describe('Task Viewer Integration - Query Invalidation', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should invalidate task, tasks list, and workspace status after sending a message', async () => {
    const taskId = 'task-123'
    const wrapper = createWrapper()

    // Setup initial data
    const initialTask = createMockTaskWithIterations({ taskId: taskId }, 1)
    vi.mocked(tasksApi.getTask).mockResolvedValue(initialTask)
    vi.mocked(tasksApi.listTasks).mockResolvedValue({
      items: [initialTask],
      pagination: { limit: 20, offset: 0, total: 1 },
    })

    // Render hooks to establish initial cache
    const { result: taskResult } = renderHook(() => useTask(taskId), {
      wrapper,
    })

    // Wait for initial data
    await waitFor(() => {
      expect(taskResult.current.data).toEqual(initialTask)
    })

    // Setup updated data after message sent
    const updatedTask = createMockTaskWithIterations(
      {
        taskId: taskId,
        status: 'running',
      },
      2,
    )
    vi.mocked(tasksApi.sendMessage).mockResolvedValue(
      createMockAgentTriggerResponse({ taskId: taskId }),
    )
    vi.mocked(tasksApi.getTask).mockResolvedValue(updatedTask)
    vi.mocked(tasksApi.listTasks).mockResolvedValue({
      items: [updatedTask],
      pagination: { limit: 20, offset: 0, total: 1 },
    })

    // Get the query client to spy on invalidations
    const client = wrapper({ children: null }).props.client
    const spyInvalidate = vi.spyOn(client, 'invalidateQueries')
    const spyRefetch = vi.spyOn(client, 'refetchQueries')

    // Send message
    const { result: sendMessageResult } = renderHook(() => useSendMessage(taskId), {
      wrapper,
    })

    await act(async () => {
      await sendMessageResult.current.mutateAsync({
        message: 'Test message',
      })
    })

    // Mutations invalidate both task lists and task-specific queries
    expect(spyInvalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.tasksList(),
    })
    expect(spyInvalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })

    // Verify immediate refetch
    expect(spyRefetch).toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })

    // Wait for data to update
    await waitFor(() => {
      expect(taskResult.current.data).toEqual(updatedTask)
    })
  })
})
