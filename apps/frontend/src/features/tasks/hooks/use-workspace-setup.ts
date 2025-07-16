import type { WorkspaceSetupRequest } from '@moatless/types'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UseWorkspaceSetupOptions {
  onSuccess?: (data: any) => void
  onError?: (error: Error) => void
}

export function useWorkspaceSetup(options?: UseWorkspaceSetupOptions) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (variables: { taskId: string; request?: WorkspaceSetupRequest }) => {
      return tasksApi.setupWorkspace(variables.taskId, variables.request)
    },
    onSuccess: (data, variables) => {
      // Invalidate workspace status to refresh the UI
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.workspaceStatus(variables.taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(variables.taskId) })
      options?.onSuccess?.(data)
    },
    onError: (error) => {
      options?.onError?.(error as Error)
    },
  })
}
