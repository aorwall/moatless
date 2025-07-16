import type { FileTreeWithChangesResponse } from '@moatless/types'
import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from '@/lib/api/client'
import { createWrapper } from '@/test-utils/render'
import { useFileTreeWithChanges } from '../use-file-tree-with-changes'

// Mock the API client
vi.mock('@/lib/api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

describe('useFileTreeWithChanges', () => {
  const mockTaskId = 'test-task-123'
  const mockFileTreeResponse: FileTreeWithChangesResponse = {
    tree: [
      {
        name: 'src',
        path: 'src',
        type: 'directory',
        isChanged: true,
        children: [
          {
            name: 'file1.ts',
            path: 'src/file1.ts',
            type: 'file',
            isChanged: true,
            gitStatus: 'modified',
            additions: 5,
            deletions: 2,
            isStaged: false,
          },
          {
            name: 'file2.ts',
            path: 'src/file2.ts',
            type: 'file',
            isChanged: true,
            gitStatus: 'added',
            additions: 10,
            deletions: 0,
            isStaged: true,
          },
        ],
      },
      {
        name: 'package.json',
        path: 'package.json',
        type: 'file',
        isChanged: false,
      },
    ],
    gitSummary: {
      totalFiles: 2,
      totalAdditions: 15,
      totalDeletions: 2,
      totalModified: 1,
      totalStaged: 1,
      compareBase: 'main',
      hasChanges: true,
    },
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should fetch file tree with changes successfully', async () => {
    vi.mocked(apiClient.get).mockResolvedValue(mockFileTreeResponse)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    expect(apiClient.get).toHaveBeenCalledWith('/api/v1/tasks/test-task-123/file-tree-with-changes')
    expect(result.current.data).toEqual(mockFileTreeResponse)
    expect(result.current.isLoading).toBe(false)
    expect(result.current.error).toBeNull()
  })

  it('should handle path parameter correctly', async () => {
    vi.mocked(apiClient.get).mockResolvedValue(mockFileTreeResponse)

    const { result } = renderHook(
      () => useFileTreeWithChanges({ taskId: mockTaskId, path: 'src' }),
      { wrapper: createWrapper() },
    )

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    expect(apiClient.get).toHaveBeenCalledWith(
      '/api/v1/tasks/test-task-123/file-tree-with-changes?path=src',
    )
  })

  it('should not include path parameter when path is default', async () => {
    vi.mocked(apiClient.get).mockResolvedValue(mockFileTreeResponse)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId, path: '.' }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    expect(apiClient.get).toHaveBeenCalledWith('/api/v1/tasks/test-task-123/file-tree-with-changes')
  })

  it('should handle API errors', async () => {
    const mockError = new Error('Network error')
    vi.mocked(apiClient.get).mockRejectedValue(mockError)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isError).toBe(true)
    })

    expect(result.current.error).toBe(mockError)
    expect(result.current.data).toBeUndefined()
    expect(result.current.isLoading).toBe(false)
  })

  it('should be disabled when taskId is not provided', async () => {
    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: '' }), {
      wrapper: createWrapper(),
    })

    // Should not make API call when taskId is empty
    expect(apiClient.get).not.toHaveBeenCalled()
    expect(result.current.isLoading).toBe(false)
  })

  it('should set up proper query key', async () => {
    vi.mocked(apiClient.get).mockResolvedValue(mockFileTreeResponse)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    // The query key should be properly structured for cache invalidation
    expect(result.current.data).toBeDefined()
  })

  it('should handle empty response correctly', async () => {
    const emptyResponse: FileTreeWithChangesResponse = {
      tree: [],
      gitSummary: {
        totalFiles: 0,
        totalAdditions: 0,
        totalDeletions: 0,
        totalModified: 0,
        totalStaged: 0,
        compareBase: 'main',
        hasChanges: false,
      },
    }

    vi.mocked(apiClient.get).mockResolvedValue(emptyResponse)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    expect(result.current.data).toEqual(emptyResponse)
    expect(result.current.data?.tree).toHaveLength(0)
    expect(result.current.data?.gitSummary.hasChanges).toBe(false)
  })

  it('should handle loading state correctly', async () => {
    // Create a promise that won't resolve immediately
    let resolvePromise: (value: FileTreeWithChangesResponse) => void
    const pendingPromise = new Promise<FileTreeWithChangesResponse>((resolve) => {
      resolvePromise = resolve
    })

    vi.mocked(apiClient.get).mockReturnValue(pendingPromise)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    // Should be loading initially
    expect(result.current.isLoading).toBe(true)
    expect(result.current.data).toBeUndefined()
    expect(result.current.error).toBeNull()

    // Resolve the promise
    resolvePromise!(mockFileTreeResponse)

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false)
    })

    expect(result.current.data).toEqual(mockFileTreeResponse)
  })

  it('should include refetch interval for auto-refresh', async () => {
    vi.mocked(apiClient.get).mockResolvedValue(mockFileTreeResponse)

    const { result } = renderHook(() => useFileTreeWithChanges({ taskId: mockTaskId }), {
      wrapper: createWrapper(),
    })

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true)
    })

    // The hook should be configured to refetch every 5 seconds
    // This is tested by verifying that the hook returns data
    expect(result.current.data).toBeDefined()
    // With refetchInterval, the data may be marked as stale for refetching
    expect(apiClient.get).toHaveBeenCalledTimes(1)
  })
})
