import type { GitnessPATRequest } from '@moatless/types'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { gitnessConnectionApi } from '@/features/settings'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * Get user's Gitness connection configuration
 */
export function useGitnessConfig() {
  return useQuery({
    queryKey: queryKeys.settings.gitness(),
    queryFn: gitnessConnectionApi.getConfig,
  })
}

/**
 * Disconnect Gitness service connection
 */
export function useDisconnectGitness(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: gitnessConnectionApi.disconnect,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.gitness() })
    },
    ...options,
  })
}

/**
 * Save Personal Access Token with base URL
 */
export function useSaveGitnessPAT(
  options?: UseMutationOptions<{ success: boolean; message: string }, ApiError, GitnessPATRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: gitnessConnectionApi.savePAT,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.gitness() })
    },
    ...options,
  })
}
