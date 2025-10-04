import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { tasksApi } from '@/features/tasks/api/tasks'
import { createMockTask } from '@/test-utils/factories'
import { createWrapper } from '@/test-utils/render'

import { useTaskData } from '../use-task-data'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    getTask: vi.fn(),
    getTaskGitDiff: vi.fn(),
    getTaskWorkspaceStatus: vi.fn(),
  },
}))

describe('useTaskData', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should fetch all task data in parallel with a single call each', async () => {
    const taskId = 'test-task-123'
    const mockTask = createMockTask({ taskId })
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
    const mockWorkspaceStatus = {
      // Workspace info
      workspaceState: 'exists' as const,
      workspacePath: '/tmp/test-workspace',
      workspaceChangeStatus: 'no_changes' as const,
      configOutdated: false,

      // Environment info
      environmentType: 'local' as const,
      environmentStatus: 'ready' as const,
      environmentCreatedAt: new Date().toISOString(),

      // Agent info
      agentState: 'idle' as const,
      agentType: 'claude-code',

      // Git status
      gitStatus: {
        currentBranch: 'test-branch',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'synced' as const,
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: true,
      },

      // Capabilities
      capabilities: {
        fileOperations: true,
        commandExecution: true,
        serverManagement: true,
      },

      // Computed states
      isRunning: false,
      workspaceReady: true,
      canExecute: true,
    }

    // Setup mock responses
    vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue(mockGitDiff)
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue(mockWorkspaceStatus)

    // Render the hook
    const { result } = renderHook(() => useTaskData(taskId), {
      wrapper: createWrapper(),
    })

    // Initially should be loading
    expect(result.current.isLoading).toBe(true)
    expect(result.current.task).toBeUndefined()
    expect(result.current.gitDiff).toBeUndefined()

    // Wait for data to load
    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    // Verify data is loaded
    expect(result.current.task).toEqual(mockTask)
    expect(result.current.gitDiff).toEqual(mockGitDiff)
    expect(result.current.workspaceStatus).toEqual(mockWorkspaceStatus)
    expect(result.current.error).toBeNull()

    // Verify each API was called exactly once
    expect(tasksApi.getTask).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTask).toHaveBeenCalledWith(taskId)

    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledWith(taskId)

    expect(tasksApi.getTaskWorkspaceStatus).toHaveBeenCalledTimes(1)
    expect(tasksApi.getTaskWorkspaceStatus).toHaveBeenCalledWith(taskId)
  })

  it('should handle errors gracefully', async () => {
    const taskId = 'test-task-123'
    const mockError = new Error('Failed to fetch task')

    // Setup mock to fail
    vi.mocked(tasksApi.getTask).mockRejectedValue(mockError)
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue({ files: [], diff: '' })
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue({
      // Workspace info
      workspaceState: 'exists' as const,
      workspacePath: '/tmp/test-workspace',
      workspaceChangeStatus: 'no_changes' as const,
      configOutdated: false,

      // Environment info
      environmentType: 'local' as const,
      environmentStatus: 'ready' as const,
      environmentCreatedAt: new Date().toISOString(),

      // Agent info
      agentState: 'idle' as const,
      agentType: 'claude-code',

      // Git status
      gitStatus: {
        currentBranch: 'test-branch',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'synced' as const,
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: true,
      },

      // Capabilities
      capabilities: {
        fileOperations: true,
        commandExecution: true,
        serverManagement: true,
      },

      // Computed states
      isRunning: false,
      workspaceReady: true,
      canExecute: true,
    })

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

  it('should still fetch data for task IDs that look temporary', async () => {
    const tempTaskId = 'temp-12345'
    const mockTask = createMockTask({ taskId: tempTaskId })

    vi.mocked(tasksApi.getTask).mockResolvedValue(mockTask)
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue({ files: [], diff: '' })
    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockResolvedValue({
      workspaceState: 'exists',
      workspacePath: '/tmp/test-workspace',
      workspaceChangeStatus: 'no_changes',
      configOutdated: false,
      environmentType: 'local',
      environmentStatus: 'ready',
      environmentCreatedAt: new Date().toISOString(),
      agentState: 'idle',
      agentType: 'claude-code',
      gitStatus: {
        currentBranch: 'main',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'synced',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: true,
      },
      capabilities: {
        fileOperations: true,
        commandExecution: true,
        serverManagement: true,
      },
      isRunning: false,
      workspaceReady: true,
      canExecute: true,
    })

    renderHook(() => useTaskData(tempTaskId), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(tasksApi.getTask).toHaveBeenCalledWith(tempTaskId)
    })

    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledWith(tempTaskId)
    expect(tasksApi.getTaskWorkspaceStatus).toHaveBeenCalledWith(tempTaskId)
  })
})
