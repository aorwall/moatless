import type { LiveStatus } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { liveStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import { queryOptions, type UseQueryOptions, useQuery } from '@tanstack/react-query'
import { queryKeys } from './query-keys'

type TaskLiveStatusQueryOptions = Omit<
  UseQueryOptions<LiveStatus, Error, LiveStatus, ReturnType<typeof queryKeys.tasks.liveStatus>>,
  'queryKey' | 'queryFn'
>

export function taskLiveStatusQueryOptions(taskId: string) {
  return queryOptions({
    queryKey: queryKeys.tasks.liveStatus(taskId),
    queryFn: () => unwrapResponse<LiveStatus>(liveStatusHandler(taskId)),
    staleTime: 5000,
    refetchInterval: false as const,
    refetchOnMount: true,
    refetchOnWindowFocus: false,
  })
}

export function useTaskLiveStatusQuery(taskId: string, options?: TaskLiveStatusQueryOptions) {
  const { enabled = true, ...restOptions } = options ?? {}

  return useQuery({
    ...taskLiveStatusQueryOptions(taskId),
    enabled: enabled && !!taskId,
    ...restOptions,
  })
}
