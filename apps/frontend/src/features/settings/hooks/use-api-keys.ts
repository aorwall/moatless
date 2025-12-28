import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { apiKeysApi } from '@/features/settings/api/api-keys.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * API Key types based on the API contract
 */
export interface ApiKey {
  id: string
  name: string
  keyPrefix: string
  scopes: string[] | null
  lastUsedAt: string | null
  expiresAt: string | null
  createdAt: string
}

export interface ApiKeyCreateResult {
  id: string
  key: string
  name: string
  keyPrefix: string
  scopes: string[] | null
  expiresAt: string | null
  createdAt: string
}

export interface CreateApiKeyRequest {
  name: string
  scopes?: string[]
  expiresInDays?: number
}

/**
 * Get all API keys for the current user
 */
export function useApiKeys() {
  return useQuery({
    queryKey: queryKeys.settings.apiKeys(),
    queryFn: apiKeysApi.list,
  })
}

/**
 * Create a new API key
 * The full key is only returned once and cannot be retrieved later
 */
export function useCreateApiKey(
  options?: UseMutationOptions<ApiKeyCreateResult, ApiError, CreateApiKeyRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: apiKeysApi.create,
    onSuccess: () => {
      // Invalidate API keys list to refresh
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.apiKeys() })
    },
    ...options,
  })
}

/**
 * Revoke an API key
 */
export function useRevokeApiKey(
  options?: UseMutationOptions<{ message: string }, ApiError, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: apiKeysApi.revoke,
    onSuccess: () => {
      // Invalidate API keys list to refresh
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.apiKeys() })
    },
    ...options,
  })
}
