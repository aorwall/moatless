import type { RouteSuccess } from '@moatless/api-contract'
import type { stopServerRoute } from '@moatless/api-contract/preview.ts'
import type { ServerStatus } from '@moatless/types'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { queryKeys } from '@/lib/api/query-keys'
import { serverApi } from '../api/server'

// Server management hooks
export function useServerStatus(
  taskId: string,
  options?: Omit<UseQueryOptions<ServerStatus>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.serverStatus(taskId),
    queryFn: () => serverApi.getServerStatus(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useStartServer(options?: UseMutationOptions<{ success: true }, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.startServer(taskId),
    onSuccess: async (data, taskId, onMutateResult, mutationContext) => {
      // Invalidate server status - SSE events will update it
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })

      options?.onSuccess?.(data, taskId, onMutateResult, mutationContext)
    },
    onError: (error, taskId, onMutateResult, mutationContext) => {
      // Handle API call failures (network errors, etc.)
      toast.error('Failed to start preview server', {
        description: error.message || 'Network error occurred',
      })
      options?.onError?.(error, taskId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useStopServer(
  options?: UseMutationOptions<RouteSuccess<typeof stopServerRoute>, Error, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.stopServer(taskId),
    onSuccess: (data, taskId, onMutateResult, mutationContext) => {
      // Only invalidate server status
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })
      options?.onSuccess?.(data, taskId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useRestartServer(options?: UseMutationOptions<{ success: true }, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.restartServer(taskId),
    onSuccess: async (data, taskId, onMutateResult, mutationContext) => {
      // Invalidate server status - SSE events will update it
      await queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })

      options?.onSuccess?.(data, taskId, onMutateResult, mutationContext)
    },
    onError: (error, taskId, onMutateResult, mutationContext) => {
      // Handle API call failures (network errors, etc.)
      toast.error('Failed to restart preview server', {
        description: error.message || 'Network error occurred',
      })
      options?.onError?.(error, taskId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useServerLogs(
  taskId: string,
  options?: Omit<UseQueryOptions<{ logs: string; lines: number }>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.serverLogs(taskId),
    queryFn: () => serverApi.getServerLogs(taskId),
    enabled: !!taskId,
    ...options,
  })
}
