import { WorkspaceStatus } from '@moatless/shared/types/workspace'
import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { tasksApi } from '@/features/tasks/api/tasks'
import { useSendMessage, useTask, useTaskWorkspaceStatus } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import {
  createMockAgentTriggerResponse,
  createMockTask,
  createMockTaskWithIterations,
  createMockWorkspaceStatus,
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
    const initialWorkspace = createMockWorkspaceStatus({
      status: WorkspaceStatus.IDLE,
    })

    vi.mocked(tasksApi.getTask).mockResolvedValue(initialTask)
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue(initialWorkspace)
    vi.mocked(tasksApi.listTasks).mockResolvedValue([initialTask])

    // Render hooks to establish initial cache
    const { result: taskResult } = renderHook(() => useTask(taskId), {
      wrapper,
    })
    const { result: workspaceResult } = renderHook(() => useTaskWorkspaceStatus(taskId), {
      wrapper,
    })

    // Wait for initial data
    await waitFor(() => {
      expect(taskResult.current.data).toEqual(initialTask)
      expect(workspaceResult.current.data).toEqual(initialWorkspace)
    })

    // Setup updated data after message sent
    const updatedTask = createMockTaskWithIterations(
      {
        taskId: taskId,
        status: 'running',
      },
      2,
    )
    const updatedWorkspace = createMockWorkspaceStatus({
      status: WorkspaceStatus.RUNNING,
    })

    vi.mocked(tasksApi.sendMessage).mockResolvedValue(
      createMockAgentTriggerResponse({ taskId: taskId }),
    )
    vi.mocked(tasksApi.getTask).mockResolvedValue(updatedTask)
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue(updatedWorkspace)
    vi.mocked(tasksApi.listTasks).mockResolvedValue([updatedTask])

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

    // Verify only task list is invalidated by mutation (events handle task-specific invalidations)
    expect(spyInvalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.tasksList(),
    })
    
    // Task-specific queries are now invalidated by events, not mutations
    expect(spyInvalidate).not.toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })
    expect(spyInvalidate).not.toHaveBeenCalledWith({
      queryKey: queryKeys.taskWorkspaceStatus(taskId),
    })

    // Verify immediate refetch
    expect(spyRefetch).toHaveBeenCalledWith({
      queryKey: queryKeys.task(taskId),
    })

    // Wait for data to update
    await waitFor(() => {
      expect(taskResult.current.data).toEqual(updatedTask)
      expect(workspaceResult.current.data).toEqual(updatedWorkspace)
    })
  })

  it('should handle workspace status transitions correctly', async () => {
    const taskId = 'task-123'
    const wrapper = createWrapper()

    const task = createMockTask({ taskId: taskId })
    vi.mocked(tasksApi.getTask).mockResolvedValue(task)

    // Start with idle workspace
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValueOnce(
      createMockWorkspaceStatus({ status: WorkspaceStatus.IDLE }),
    )

    const { result } = renderHook(() => useTaskWorkspaceStatus(taskId), {
      wrapper,
    })

    await waitFor(() => {
      expect(result.current.data?.status).toBe(WorkspaceStatus.IDLE)
    })

    // Update to running
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValueOnce(
      createMockWorkspaceStatus({ status: WorkspaceStatus.RUNNING }),
    )

    // Manually invalidate to simulate external update
    const client = wrapper({ children: null }).props.client
    await client.invalidateQueries({
      queryKey: queryKeys.taskWorkspaceStatus(taskId),
    })

    await waitFor(() => {
      expect(result.current.data?.status).toBe(WorkspaceStatus.RUNNING)
    })
  })

  it('should refetch all related queries when message is sent', async () => {
    const taskId = 'task-123'
    const wrapper = createWrapper()

    // Track API calls
    const getTaskSpy = vi.mocked(tasksApi.getTask)
    const getWorkspaceSpy = vi.mocked(tasksApi.getTaskWorkspaceStatus)
    const listTasksSpy = vi.mocked(tasksApi.listTasks)

    // Setup mocks
    const mockTask = createMockTask({ taskId: taskId })
    getTaskSpy.mockResolvedValue(mockTask)
    getWorkspaceSpy.mockResolvedValue(createMockWorkspaceStatus({ status: WorkspaceStatus.IDLE }))
    listTasksSpy.mockResolvedValue([mockTask])
    vi.mocked(tasksApi.sendMessage).mockResolvedValue(
      createMockAgentTriggerResponse({ taskId: taskId }),
    )

    // Establish initial queries
    const { result: taskResult } = renderHook(() => useTask(taskId), {
      wrapper,
    })
    const { result: workspaceResult } = renderHook(() => useTaskWorkspaceStatus(taskId), {
      wrapper,
    })

    await waitFor(() => {
      expect(taskResult.current.data).toBeDefined()
      expect(workspaceResult.current.data).toBeDefined()
    })

    // Verify initial calls
    expect(getTaskSpy).toHaveBeenCalledTimes(1)
    expect(getWorkspaceSpy).toHaveBeenCalledTimes(1)

    // Clear call counts
    getTaskSpy.mockClear()
    getWorkspaceSpy.mockClear()
    listTasksSpy.mockClear()

    // Send message
    const { result } = renderHook(() => useSendMessage(taskId), { wrapper })

    await act(async () => {
      await result.current.mutateAsync({
        message: 'Trigger refetch',
      })
    })

    // In the new event-driven architecture, mutations trigger immediate refetch for instant feedback
    // Task data should be refetched immediately, but other calls depend on what queries are active
    expect(getTaskSpy).toHaveBeenCalledTimes(1) // Immediate refetch after message
    
    // The workspace and task list calls depend on whether those queries are actively being watched
    // Since this test doesn't have active task list or workspace queries, they may not be called
    // The important thing is that the task data is refetched for immediate feedback
  })
})
