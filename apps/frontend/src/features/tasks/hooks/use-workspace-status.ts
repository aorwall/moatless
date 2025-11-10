import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useTaskWorkspaceStatusQuery } from './use-task-queries'

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
    enabled: isEnabled && task?.status === 'open',
  } as Parameters<typeof useTaskWorkspaceStatusQuery>[1])
}
