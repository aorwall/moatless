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
    refetchInterval: (query) => {
      // Poll more frequently when server is running or initializing
      const data = query.state.data
      if (data?.running || data?.status === 'initializing') {
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
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })

      // Show success or error toast based on server status
      if (data.running) {
        // Success case - server started successfully
        toast.success('Preview server started successfully')
      } else if (data.error) {
        // Failed case - show error details in toast
        toast.error('Failed to start preview server', {
          description: data.error,
        })
        // Don't throw an error since this is an expected state now
      }

      options?.onSuccess?.(data, taskId, context)
    },
    onError: (error, taskId, context) => {
      // Handle API call failures (network errors, etc.)
      toast.error('Failed to start preview server', {
        description: error.message || 'Network error occurred',
      })
      options?.onError?.(error, taskId, context)
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
        queryKey: queryKeys.tasks.serverStatus(taskId),
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
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })
      // Force refetch the server status immediately
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.serverStatus(taskId),
      })

      // Show success or error toast based on server status
      if (data.running) {
        toast.success('Preview server restarted successfully')
      } else if (data.error) {
        toast.error('Failed to restart preview server', {
          description: data.error,
        })
      }

      options?.onSuccess?.(data, taskId, context)
    },
    onError: (error, taskId, context) => {
      // Handle API call failures (network errors, etc.)
      toast.error('Failed to restart preview server', {
        description: error.message || 'Network error occurred',
      })
      options?.onError?.(error, taskId, context)
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
