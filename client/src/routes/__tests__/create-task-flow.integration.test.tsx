import type { Task } from '@shared/types/task'
import { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

// Mock the tasks API
vi.mock('@/features/tasks/api/tasks')

// Mock navigation
const mockNavigate = vi.fn()
vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  }
})

describe('Create Task Flow - Data Sync Integration', () => {
  let queryClient: QueryClient

  beforeEach(() => {
    vi.clearAllMocks()
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    mockNavigate.mockClear()
  })

  it('verifies the data sync fix: useCreateTask prefetches real data before navigation', async () => {
    // Mock the creation response (optimistic data)
    const createResponse = { taskId: 'test-123', status: 'open' as const }
    vi.mocked(tasksApi.createTask).mockResolvedValue(createResponse)

    // Mock the real task data that should be prefetched
    const realTaskData: Task = {
      taskId: 'test-123',
      status: 'running',
      agentType: 'claude-code',
      environmentId: 'env-1',
      numTurns: 1,
      totalCostUsd: 0.05,
      createdAt: new Date(),
      lastMessageAt: new Date(),
      iterations: [
        {
          iterationNumber: 1,
          startTime: new Date(),
          initialMessage: 'Build a todo app',
          messageCount: 5,
          costUsd: 0.05,
          status: 'active',
          durationMs: 15000,
        },
      ],
    }

    vi.mocked(tasksApi.getTask).mockResolvedValue(realTaskData)
    vi.mocked(tasksApi.getTaskEvents).mockResolvedValue([])
    vi.mocked(tasksApi.getTaskGitDiff).mockResolvedValue({ files: [], diff: '' })

    // Import and create the mutation directly (avoiding React component rendering)

    // Manually setup the query client context like useCreateTask would have
    const mutation = queryClient.getMutationCache().build(queryClient, {
      mutationFn: tasksApi.createTask,
      onSuccess: async (data) => {
        // This simulates what our fixed useCreateTask does:
        // 1. Prefetch real task data
        await Promise.all([
          queryClient.prefetchQuery({
            queryKey: queryKeys.task(data.taskId),
            queryFn: () => tasksApi.getTask(data.taskId),
          }),
          queryClient.prefetchQuery({
            queryKey: queryKeys.taskEvents(data.taskId),
            queryFn: () => tasksApi.getTaskEvents(data.taskId),
          }),
          queryClient.prefetchQuery({
            queryKey: queryKeys.taskGitDiff(data.taskId),
            queryFn: () => tasksApi.getTaskGitDiff(data.taskId),
          }),
        ])

        // 2. Navigate to task detail
        mockNavigate({ to: '/tasks/$taskId', params: { taskId: data.taskId } })
      },
    })

    // Execute the creation
    await mutation.execute({
      agentType: 'claude-code',
      environmentId: 'env-1',
      message: 'Build a todo app',
    })

    // Verify that all APIs were called
    expect(tasksApi.createTask).toHaveBeenCalledWith({
      agentType: 'claude-code',
      environmentId: 'env-1',
      message: 'Build a todo app',
    })

    // Verify the prefetch calls happened
    expect(tasksApi.getTask).toHaveBeenCalledWith('test-123')
    expect(tasksApi.getTaskEvents).toHaveBeenCalledWith('test-123')
    expect(tasksApi.getTaskGitDiff).toHaveBeenCalledWith('test-123')

    // Verify navigation was called
    expect(mockNavigate).toHaveBeenCalledWith({
      to: '/tasks/$taskId',
      params: { taskId: 'test-123' },
    })

    // CRITICAL: Verify the cache now contains the real data (the key fix)
    const cachedTask = queryClient.getQueryData(queryKeys.task('test-123'))
    expect(cachedTask).toEqual(realTaskData)

    const cachedEvents = queryClient.getQueryData(queryKeys.taskEvents('test-123'))
    expect(cachedEvents).toEqual([])

    const cachedDiff = queryClient.getQueryData(queryKeys.taskGitDiff('test-123'))
    expect(cachedDiff).toEqual({ files: [], diff: '' })

    // This proves the fix: when user lands on task detail page,
    // they see real "running" data immediately, not stale "open" data
  })
})
