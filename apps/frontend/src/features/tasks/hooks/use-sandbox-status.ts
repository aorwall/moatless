import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import {
  cleanupHandler,
  provisionHandler,
  redeployHandler,
  restartHandler,
  stopHandler,
} from '@moatless/api-client/sandbox/sandbox/sandbox'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskSandboxStatusQuery } from './use-task-queries'

interface UseSandboxStatusOptions
  extends Omit<UseQueryOptions<SandboxStatusFullResponse>, 'queryKey' | 'queryFn'> {
  enabled?: boolean
}

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
  const isEnabled = options?.enabled ?? true

  return useTaskSandboxStatusQuery(taskId, {
    ...options,
    enabled: isEnabled && task?.status !== 'closed',
  } as Parameters<typeof useTaskSandboxStatusQuery>[1])
}

/**
 * Hook to retry sandbox provisioning after a failure
 * Calls backend's provision endpoint directly
 * Note: Status updates are handled by SSE events in global-event-stream-provider
 */
export function useSandboxRetry(taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => provisionHandler(taskId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.sandboxStatus(taskId) })
    },
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
    },
  })
}
