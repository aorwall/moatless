import {
  restartHandler,
  startHandler,
  stopHandler,
} from '@moatless/api-client/sandbox/sandbox/sandbox'
import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskWorkspaceStatusQuery } from './use-task-queries'

interface UseWorkspaceStatusOptions
  extends Omit<UseQueryOptions<WorkspaceStatusResponse>, 'queryKey' | 'queryFn'> {
  enabled?: boolean
}

/**
 * Hook to fetch workspace status for a task
 * - Fetches for active tasks (open, running, error)
 * - Stops fetching for terminal states (closed, merged)
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
    enabled: isEnabled && task?.status !== 'closed' && task?.status !== 'merged',
  } as Parameters<typeof useTaskWorkspaceStatusQuery>[1])
}

/**
 * Hook to retry workspace provisioning after a failure
 * Calls backend's start endpoint directly
 * Note: Status updates are handled by SSE events in global-event-stream-provider
 */
export function useWorkspaceRetry(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => startHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(taskId) })
    },
  })
}

/**
 * Hook to restart workspace via backend (Deployment pod restart)
 * Preserves workspace data (PVC in K8s)
 */
export function useWorkspaceRestart(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => restartHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(taskId) })
    },
  })
}

/**
 * Hook to stop workspace via backend (scale Deployment to 0)
 */
export function useWorkspaceStop(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => stopHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(taskId) })
    },
  })
}

/**
 * Hook to start a stopped workspace via backend (scale Deployment to 1)
 */
export function useWorkspaceStart(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => startHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(taskId) })
    },
  })
}
