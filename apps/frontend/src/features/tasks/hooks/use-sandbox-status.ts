import type { TaskResponse } from '@moatless/api-client'
import {
  cleanupHandler,
  redeployHandler,
  restartHandler,
  stopHandler,
} from '@moatless/api-client/sandbox/sandbox/sandbox'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'

// Re-export the shared retry mutation so existing tasks-side callers keep their
// `@/features/tasks/hooks/use-sandbox-status` import.
export { useSandboxRetry } from '@/lib/api/task-sandbox-queries'

type UseSandboxStatusOptions = Parameters<typeof useTaskSandboxStatusQuery>[1]

/**
 * Hook to fetch sandbox status for a task
 * - Fetches for active tasks (open, running, error)
 * - Stops fetching for terminal states (closed, merged)
 * - Polls intelligently based on environment/agent state
 * - Provides derived states for components
 */
export function useSandboxStatus(
  taskId: string,
  task?: TaskResponse,
  options?: UseSandboxStatusOptions,
) {
  const { enabled, ...restOptions } = options ?? {}
  const isEnabled = enabled ?? true

  return useTaskSandboxStatusQuery(taskId, {
    ...restOptions,
    enabled: isEnabled && task?.status !== 'closed',
  })
}

/**
 * Hook to restart sandbox via backend (Deployment pod restart)
 * Preserves workspace data (PVC in K8s)
 */
export function useSandboxRestart(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => restartHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.sandboxStatus(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.liveStatus(taskId) })
    },
  })
}

/**
 * Hook to stop sandbox via backend (scale Deployment to 0)
 */
export function useSandboxStop(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => stopHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.sandboxStatus(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.liveStatus(taskId) })
    },
  })
}

/**
 * Hook to redeploy sandbox (delete Deployment/Service/IngressRoutes, recreate with fresh config)
 * Preserves workspace data (PVC)
 */
export function useSandboxRedeploy(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => redeployHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.sandboxStatus(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.liveStatus(taskId) })
    },
  })
}

/**
 * Hook to clean up sandbox and all associated resources
 */
export function useSandboxCleanup(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => cleanupHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.sandboxStatus(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.liveStatus(taskId) })
    },
  })
}
