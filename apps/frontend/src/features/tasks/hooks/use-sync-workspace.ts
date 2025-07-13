import { type UseMutationOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { type ApiError, apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

interface SyncWorkspaceRequest {
  taskId: string
  baseBranch?: string
}

interface SyncWorkspaceResponse {
  success: boolean
  conflicts: boolean
  message: string
  commitsBehind?: number
  commitsAhead?: number
}

async function syncWorkspace({
  taskId,
  baseBranch,
}: SyncWorkspaceRequest): Promise<SyncWorkspaceResponse> {
  const url = baseBranch
    ? `/api/v1/tasks/${taskId}/sync-with-main?baseBranch=${encodeURIComponent(baseBranch)}`
    : `/api/v1/tasks/${taskId}/sync-with-main`

  return await apiClient.post<SyncWorkspaceResponse>(url)
}

export function useSyncWorkspace(
  options?: UseMutationOptions<SyncWorkspaceResponse, ApiError, SyncWorkspaceRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<SyncWorkspaceResponse, ApiError, SyncWorkspaceRequest>({
    mutationFn: syncWorkspace,
    onSuccess: (data, variables, context) => {
      // Invalidate workspace status and git status queries
      queryClient.invalidateQueries({ queryKey: queryKeys.taskWorkspaceStatus(variables.taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.task(variables.taskId) })

      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}
