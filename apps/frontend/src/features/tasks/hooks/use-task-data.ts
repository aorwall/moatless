import type { GitDiffResponse, Task, TaskIteration, WorkspaceStatusResponse } from '@moatless/types'
import {
  useTaskDetailQuery,
  useTaskGitDiffQuery,
  useTaskWorkspaceStatusQuery,
} from './use-task-queries'

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
  const taskQuery = useTaskDetailQuery(taskId, {
    staleTime: 5 * 60 * 1000,
  })

  const gitDiffQuery = useTaskGitDiffQuery(taskId, {
    staleTime: 10 * 60 * 1000,
    retry: false,
  })

  const workspaceStatusQuery = useTaskWorkspaceStatusQuery(taskId, {
    // Removed polling - rely on event-driven updates when aggregating task data
    refetchInterval: false,
    staleTime: 5 * 60 * 1000,
  })

  const task = taskQuery.data as Task | undefined
  const gitDiff = gitDiffQuery.data as GitDiffResponse | undefined
  const workspaceStatus = workspaceStatusQuery.data as WorkspaceStatusResponse | undefined

  const isLoading = taskQuery.isLoading || gitDiffQuery.isLoading || workspaceStatusQuery.isLoading
  const isFetching =
    taskQuery.isFetching || gitDiffQuery.isFetching || workspaceStatusQuery.isFetching

  const error = [taskQuery.error, gitDiffQuery.error, workspaceStatusQuery.error].find(
    (e): e is Error => e instanceof Error,
  )

  return {
    task,
    iterations: task?.iterations || [], // Extract iterations from task data
    gitDiff,
    workspaceStatus,
    isLoading: isLoading || isFetching,
    error: error ?? null,
  }
}
