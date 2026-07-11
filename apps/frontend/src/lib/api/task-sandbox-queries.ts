import type { SandboxStatusFullResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  provisionHandler,
  sandboxStatusHandler,
} from '@moatless/api-client/sandbox/sandbox/sandbox'
import { type UseQueryOptions, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from './query-keys'

/**
 * Shared task sandbox-status data hooks.
 *
 * These live in `lib/api` (not a feature) because both `features/tasks` and
 * `features/files` need them: they only depend on the generated API client and
 * the shared query keys, so keeping them here avoids a `files → tasks` import
 * cycle. `features/tasks` re-exports them for its own callers.
 */

type TaskSandboxStatusQueryOptions = Omit<
  UseQueryOptions<
    SandboxStatusFullResponse,
    Error,
    SandboxStatusFullResponse,
    ReturnType<typeof queryKeys.tasks.sandboxStatus>
  >,
  'queryKey' | 'queryFn'
>

export function useTaskSandboxStatusQuery(taskId: string, options?: TaskSandboxStatusQueryOptions) {
  const {
    enabled = true,
    staleTime = 5000,
    refetchInterval = false,
    ...queryOptions
  } = options ?? {}

  // Polling disabled by default - rely on SSE events for real-time updates
  // Callers can override with refetchInterval if polling is specifically needed

  return useQuery({
    queryKey: queryKeys.tasks.sandboxStatus(taskId),
    queryFn: () => unwrapResponse<SandboxStatusFullResponse>(sandboxStatusHandler(taskId)),
    enabled: enabled && !!taskId,
    staleTime,
    refetchInterval,
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
    ...queryOptions,
  })
}

/**
 * Hook to retry sandbox provisioning after a failure.
 * Calls the backend's provision endpoint directly.
 * Note: Status updates are handled by SSE events in global-event-stream-provider.
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
