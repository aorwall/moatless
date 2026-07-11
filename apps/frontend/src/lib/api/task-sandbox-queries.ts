import type { SandboxStatusFullResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  provisionHandler,
  sandboxStatusHandler,
} from '@moatless/api-client/sandbox/sandbox/sandbox'
import {
  queryOptions,
  type UseQueryOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
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

/**
 * Colocated `queryKey` + `queryFn` (+ defaults) for a task's sandbox status.
 * Use this with `getQueryData`/`ensureQueryData`/`setQueryData` for typed,
 * key-in-sync cache access (route loaders, the SSE provider, the dashboard cell).
 */
export function taskSandboxStatusQueryOptions(taskId: string) {
  return queryOptions({
    queryKey: queryKeys.tasks.sandboxStatus(taskId),
    queryFn: () => unwrapResponse<SandboxStatusFullResponse>(sandboxStatusHandler(taskId)),
    // Polling disabled by default - rely on SSE events for real-time updates.
    staleTime: 5000,
    refetchInterval: false as const,
    refetchOnMount: true, // Only refetch if stale
    refetchOnWindowFocus: false, // SSE handles updates
  })
}

export function useTaskSandboxStatusQuery(taskId: string, options?: TaskSandboxStatusQueryOptions) {
  const { enabled = true, ...restOptions } = options ?? {}

  // Callers can override staleTime/refetchInterval via `options` if polling is
  // specifically needed; the base config comes from `taskSandboxStatusQueryOptions`.
  return useQuery({
    ...taskSandboxStatusQueryOptions(taskId),
    enabled: enabled && !!taskId,
    ...restOptions,
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
