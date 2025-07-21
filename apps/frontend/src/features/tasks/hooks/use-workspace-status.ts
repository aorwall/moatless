import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

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
  const shouldFetch =
    isEnabled && !!taskId && !taskId.startsWith('temp-') && task?.status === 'open'

  const query = useQuery({
    queryKey: queryKeys.tasks.workspaceStatus(taskId),
    queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
    enabled: shouldFetch,
    refetchInterval: (query) => {
      const data = query.state.data as WorkspaceStatusResponse | undefined
      const envStatus = data?.environmentStatus
      const agentState = data?.agentState

      // Only poll when in transitional states
      if (envStatus === 'initializing' || envStatus === 'cleaning' || agentState === 'running') {
        return 3000 // 3 seconds for transitional states
      }
      return false // Stop polling when stable
    },
    staleTime: 2000, // Consider data stale after 2 seconds
    ...options,
  })

  return query
}

/**
 * Hook to fetch workspace status with automatic task fetching
 * Useful when you only have the taskId
 */
export function useWorkspaceStatusWithTask(taskId: string, options?: UseWorkspaceStatusOptions) {
  // First fetch the task to get its status
  const taskQuery = useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-'),
    staleTime: 30000, // 30s - task metadata doesn't change frequently
  })

  // Then use the workspace status hook with the task
  return useWorkspaceStatus(taskId, taskQuery.data, {
    ...options,
    // Only fetch workspace status if task is loaded and open
    enabled: (options?.enabled ?? true) && taskQuery.isSuccess && taskQuery.data?.status === 'open',
  })
}
