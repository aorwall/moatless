import type { Event } from '@shared/types/event'
import type { GitDiffResponse, Task } from '@shared/types/task'
import { useQueries } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UseTaskDataResult {
  task: Task | undefined
  events: Event[]
  gitDiff: GitDiffResponse | undefined
  isLoading: boolean
  error: Error | null
}

/**
 * Hook that fetches all task data in parallel for better performance.
 * This avoids waterfall loading by using React Query's useQueries.
 */
export function useTaskData(taskId: string): UseTaskDataResult {
  const queries = useQueries({
    queries: [
      {
        queryKey: queryKeys.task(taskId),
        queryFn: () => tasksApi.getTask(taskId),
        staleTime: 5 * 60 * 1000, // 5 minutes
        enabled: !!taskId && !taskId.startsWith('temp-'),
      },
      {
        queryKey: queryKeys.taskEvents(taskId),
        queryFn: () => tasksApi.getTaskEvents(taskId),
        staleTime: 60 * 1000, // 1 minute
        enabled: !!taskId && !taskId.startsWith('temp-'),
      },
      {
        queryKey: queryKeys.taskGitDiff(taskId),
        queryFn: () => tasksApi.getTaskGitDiff(taskId),
        staleTime: 10 * 60 * 1000, // 10 minutes
        enabled: !!taskId && !taskId.startsWith('temp-'),
        // Git diff is optional - don't fail if it doesn't exist
        retry: false,
      },
    ],
  })

  return {
    task: queries[0].data,
    events: queries[1].data || [],
    gitDiff: queries[2].data,
    isLoading: queries.some((q) => q.isLoading && q.isFetching),
    error: queries.find((q) => q.error)?.error || null,
  }
}

/**
 * Prefetch task data for navigation.
 * Call this before navigating to a task detail page.
 */
export async function prefetchTaskData(taskId: string, queryClient: any): Promise<void> {
  await Promise.all([
    queryClient.prefetchQuery({
      queryKey: queryKeys.task(taskId),
      queryFn: () => tasksApi.getTask(taskId),
      staleTime: 5 * 60 * 1000,
    }),
    queryClient.prefetchQuery({
      queryKey: queryKeys.taskEvents(taskId),
      queryFn: () => tasksApi.getTaskEvents(taskId),
      staleTime: 60 * 1000,
    }),
  ])
}
