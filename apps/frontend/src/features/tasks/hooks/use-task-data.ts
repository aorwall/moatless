import type { WorkspaceStatusResponse } from '@moatless/shared/types/api'
import type { Event } from '@moatless/shared/types/event'
import type { GitDiffResponse, Task, TaskIteration } from '@moatless/shared/types/task'
import { useQueries } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UseTaskDataResult {
  task: Task | undefined
  events: Event[]
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
  const queries = useQueries({
    queries: [
      {
        queryKey: queryKeys.task(taskId),
        queryFn: () => tasksApi.getTask(taskId),
        staleTime: 5 * 60 * 1000,
        enabled: !!taskId,
      },
      {
        queryKey: queryKeys.taskEvents(taskId),
        queryFn: () => tasksApi.getTaskEvents(taskId),
        staleTime: 60 * 1000,
        enabled: !!taskId,
      },
      {
        queryKey: queryKeys.taskGitDiff(taskId),
        queryFn: () => tasksApi.getTaskGitDiff(taskId),
        staleTime: 10 * 60 * 1000,
        enabled: !!taskId,
        retry: false,
      },
      {
        queryKey: queryKeys.taskWorkspaceStatus(taskId),
        queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
        enabled: !!taskId,
        refetchInterval: 10000, // 10 seconds - balanced polling frequency
        staleTime: 2000, // Consider data stale after 2 seconds
      },
    ],
  })

  const [taskQuery, eventsQuery, gitDiffQuery, workspaceStatusQuery] = queries

  const task = taskQuery.data as Task | undefined
  const events = (eventsQuery.data as Event[]) || []
  const gitDiff = gitDiffQuery.data as GitDiffResponse | undefined
  const workspaceStatus = workspaceStatusQuery.data as WorkspaceStatusResponse | undefined

  return {
    task,
    events,
    iterations: task?.iterations || [], // Extract iterations from task data
    gitDiff,
    workspaceStatus,
    isLoading: queries.some((q) => q.isLoading || q.isFetching),
    error: queries.find((q) => q.error)?.error || null,
  }
}
