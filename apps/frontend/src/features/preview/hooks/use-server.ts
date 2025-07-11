import type { ServerStatus } from '@moatless/shared/types/workspace'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import { serverApi } from '../api/server'

// Server management hooks
export function useServerStatus(
  taskId: string,
  options?: Omit<UseQueryOptions<ServerStatus>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskServerStatus(taskId),
    queryFn: () => serverApi.getServerStatus(taskId),
    enabled: !!taskId,
    refetchInterval: (query) => {
      // Poll more frequently when server is running
      const data = query.state.data
      if (data?.running) {
        return 5000 // 5 seconds
      }
      return false
    },
    ...options,
  })
}

export function useStartServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.startServer(taskId),
    onSuccess: async (data, taskId, context) => {
      // Only invalidate server status
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })

      // Show success or error toast based on server status
      if (data.running) {
        // Success case - server started successfully
      } else if (data.error) {
        // Failed case - show error details in toast
        // Don't throw an error since this is an expected state now
      }

      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useStopServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.stopServer(taskId),
    onSuccess: (data, taskId, context) => {
      // Only invalidate server status
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useRestartServer(options?: UseMutationOptions<ServerStatus, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (taskId: string) => serverApi.restartServer(taskId),
    onSuccess: async (data, taskId, context) => {
      // Only invalidate server status
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.taskServerStatus(taskId),
      })
      options?.onSuccess?.(data, taskId, context)
    },
    ...options,
  })
}

export function useServerLogs(
  taskId: string,
  options?: Omit<UseQueryOptions<{ logs: string; lines: number }>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskServerLogs(taskId),
    queryFn: () => serverApi.getServerLogs(taskId),
    enabled: !!taskId,
    refetchInterval: 2000, // Poll every 2 seconds
    ...options,
  })
}
