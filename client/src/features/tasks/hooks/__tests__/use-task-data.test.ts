import { EventType } from '@shared/types/event'
import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { tasksApi } from '@/features/tasks/api/tasks'
import { createMockEvent, createMockTask } from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'

import { useTaskData } from '../use-task-data'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    getTask: vi.fn(),
    getTaskEvents: vi.fn(),
    getTaskGitDiff: vi.fn(),
  },
}))

describe('useTaskData', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should fetch all task data in parallel with a single call each', async () => {
    const taskId = 'test-task-123'
    const mockTask = createMockTask({ taskId })
    const mockEvents = [
      createMockEvent({ taskId, type: EventType.MESSAGE }),
      createMockEvent({ taskId, type: EventType.TOOL }),
    ]
    const mockGitDiff = {
      diff: 'diff --git a/file.ts b/file.ts\n+added line',
      files: [
        {
          oldFile: 'file.ts',
          newFile: 'file.ts',
          hunks: ['@@ -1,1 +1,2 @@\n-old line\n+new line\n+added line'],
        },
      ],
    }

    // Setup mock responses
    vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)
    vi.mocked(tasksApi.getTaskEvents).mockResolvedValue(mockEvents)
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue(mockGitDiff)

    // Render the hook
    const { result } = renderHook(() => useTaskData(taskId), {
      wrapper: createWrapper(),
    })

    // Initially should be loading
    expect(result.current.isLoading).toBe(true)
    expect(result.current.task).toBeUndefined()
    expect(result.current.events).toEqual([])
    expect(result.current.gitDiff).toBeUndefined()

    // Wait for data to load
    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    // Verify data is loaded
    expect(result.current.task).toEqual(mockTask)
    expect(result.current.events).toEqual(mockEvents)
    expect(result.current.gitDiff).toEqual(mockGitDiff)
    expect(result.current.error).toBeNull()

    // Verify each API was called exactly once
    expect(tasksApi.getTask).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTask).toHaveBeenCalledWith(taskId)

    expect(tasksApi.getTaskEvents).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTaskEvents).toHaveBeenCalledWith(taskId)

    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledWith(taskId)
  })

  it('should handle errors gracefully', async () => {
    const taskId = 'test-task-123'
    const mockError = new Error('Failed to fetch task')

    // Setup mock to fail
    vi.mocked(tasksApi.getTask).mockRejectedValue(mockError)
    vi.mocked(tasksApi.getTaskEvents).mockResolvedValue([])
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue(undefined as any)

    // Render the hook
    const { result } = renderHook(() => useTaskData(taskId), {
      wrapper: createWrapper(),
    })

    // Wait for error
    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    // Verify error state
    expect(result.current.error).toEqual(mockError)
    expect(result.current.task).toBeUndefined()
  })

  it('should not fetch data for temporary task IDs', () => {
    const tempTaskId = 'temp-12345'

    // Render the hook with temp ID
    renderHook(() => useTaskData(tempTaskId), {
      wrapper: createWrapper(),
    })

    // APIs should not be called
    expect(tasksApi.getTask).not.toHaveBeenCalled()
    expect(tasksApi.getTaskEvents).not.toHaveBeenCalled()
    expect(tasksApi.getTaskGitDiff).not.toHaveBeenCalled()
  })
})
