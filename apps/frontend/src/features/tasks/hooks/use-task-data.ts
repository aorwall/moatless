import type { GitDiffResponse, Task, TaskIteration, WorkspaceStatusResponse } from '@moatless/types'
import { useQueries } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UseTaskDataResult {
  task: Task | undefined
  iterations: TaskIteration[]
  gitDiff: GitDiffResponse | undefined
  workspaceStatus: WorkspaceStatusResponse | undefined
  isLoading: boolean
  error: Error | null
}

/**
 * Hook that fetches all task data in parallel for better performance.
 * This avoids waterfall loading by using React Query's useQueries.
 *
 * Uses the new hierarchical query key structure where sub-resources
 * are automatically invalidated when the parent task is invalidated.
 */
export function useTaskData(taskId: string): UseTaskDataResult {
  // Don't fetch data for temporary task IDs
  const isTemporaryId = taskId.startsWith('temp-')
  const shouldFetch = !!taskId && !isTemporaryId

  const queries = useQueries({
    queries: [
      {
        queryKey: queryKeys.tasks.detail(taskId),
        queryFn: () => tasksApi.getTask(taskId),
        staleTime: 5 * 60 * 1000,
        enabled: shouldFetch,
      },
      {
        queryKey: queryKeys.tasks.gitDiff(taskId),
        queryFn: () => tasksApi.getTaskGitDiff(taskId),
        staleTime: 10 * 60 * 1000,
        enabled: shouldFetch,
        retry: false,
      },
      {
        queryKey: queryKeys.tasks.workspaceStatus(taskId),
        queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
        enabled: shouldFetch,
        // Removed polling - rely on event-driven updates
        staleTime: 5 * 60 * 1000, // Increase stale time since we have real-time updates
      },
    ],
  })

  const [taskQuery, gitDiffQuery, workspaceStatusQuery] = queries

  const task = taskQuery.data as Task | undefined
  const gitDiff = gitDiffQuery.data as GitDiffResponse | undefined
  const workspaceStatus = workspaceStatusQuery.data as WorkspaceStatusResponse | undefined

  return {
    task,
    iterations: task?.iterations || [], // Extract iterations from task data
    gitDiff,
    workspaceStatus,
    isLoading: queries.some((q) => q.isLoading || q.isFetching),
    error: queries.find((q) => q.error)?.error || null,
  }
}
