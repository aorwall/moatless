import type {
  CreateLoopRequest,
  Loop,
  LoopRecentTaskItem,
  UpdateLoopRequest,
} from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { loopsApi } from '@/features/loops/api/loops.api'
import { queryKeys } from '@/lib/api/query-keys'

export function useLoops(options?: Omit<UseQueryOptions<Loop[]>, 'queryKey' | 'queryFn'>) {
  return useQuery({
    queryKey: queryKeys.loops.list(),
    queryFn: () => loopsApi.listLoops(),
    ...options,
  })
}

export function useCreateLoop() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateLoopRequest) => loopsApi.createLoop(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.loops.all() })
    },
  })
}

export function useUpdateLoop() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateLoopRequest }) =>
      loopsApi.updateLoop(id, request),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.loops.all() })
      queryClient.invalidateQueries({
        queryKey: queryKeys.loops.detail(variables.id),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.loops.recentTasks(variables.id),
      })
    },
  })
}

export function useDeleteLoop() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (id: string) => loopsApi.deleteLoop(id),
    onSuccess: (_data, id) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.loops.all() })
      queryClient.invalidateQueries({ queryKey: queryKeys.loops.detail(id) })
      queryClient.invalidateQueries({
        queryKey: queryKeys.loops.recentTasks(id),
      })
    },
  })
}

export function useLoop(loopId: string) {
  return useQuery({
    queryKey: queryKeys.loops.detail(loopId),
    queryFn: () => loopsApi.getLoop(loopId),
    enabled: !!loopId,
  })
}

export function useLoopRecentTasks(loopId: string, opts?: { limit?: number }) {
  return useQuery<LoopRecentTaskItem[]>({
    queryKey: queryKeys.loops.recentTasks(loopId),
    queryFn: () => loopsApi.listLoopRecentTasks(loopId, { limit: opts?.limit ?? 10 }),
    enabled: !!loopId,
  })
}
