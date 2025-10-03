import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useTaskDetailQuery, useTaskWorkspaceStatusQuery } from './use-task-queries'

interface UseWorkspaceStatusOptions
  extends Omit<UseQueryOptions<WorkspaceStatusResponse>, 'queryKey' | 'queryFn'> {
  enabled?: boolean
}

/**
 * Hook to fetch workspace status for a task
 * - Only fetches when task status is "open" (not closed/merged)
 * - Polls intelligently based on environment/agent state
 * - Provides derived states for components
 */
export function useWorkspaceStatus(
  taskId: string,
  task?: Task,
  options?: UseWorkspaceStatusOptions,
) {
  const isEnabled = options?.enabled ?? true

  return useTaskWorkspaceStatusQuery(taskId, {
    ...options,
    enabled: isEnabled && !!task && task.status === 'open',
  })
}

/**
 * Hook to fetch workspace status with automatic task fetching
 * Useful when you only have the taskId
 */
export function useWorkspaceStatusWithTask(taskId: string, options?: UseWorkspaceStatusOptions) {
  // First fetch the task to get its status
  const taskQuery = useTaskDetailQuery(taskId)

  // Then use the workspace status hook with the task
  return useWorkspaceStatus(taskId, taskQuery.data, {
    ...options,
    // Only fetch workspace status if task is loaded and open
    enabled: (options?.enabled ?? true) && taskQuery.isSuccess && taskQuery.data?.status === 'open',
  })
}
