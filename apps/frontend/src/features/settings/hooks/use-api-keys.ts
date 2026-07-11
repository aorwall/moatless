import type {
  ApiKeyCreateResponse,
  ApiKeyListItem,
  CreateApiKeyRequest,
} from '@moatless/api-client'
import type { ErrorType } from '@moatless/api-client/custom-instance'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createApiKeyHandler,
  listApiKeysHandler,
  revokeApiKeyHandler,
} from '@moatless/api-client/sandbox/api-keys/api-keys'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

// Re-export generated types for consumers
export type {
  ApiKeyCreateResponse as ApiKeyCreateResult,
  ApiKeyListItem as ApiKey,
  CreateApiKeyRequest,
} from '@moatless/api-client'

/**
 * Get all API keys for the current user
 */
export function useApiKeys() {
  return useQuery({
    queryKey: queryKeys.settings.apiKeys(),
    queryFn: () => unwrapResponse<ApiKeyListItem[]>(listApiKeysHandler()),
  })
}

/**
 * Create a new API key
 * The full key is only returned once and cannot be retrieved later
 */
export function useCreateApiKey(
  options?: UseMutationOptions<ApiKeyCreateResponse, ErrorType<unknown>, CreateApiKeyRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateApiKeyRequest) =>
      unwrapResponse<ApiKeyCreateResponse>(createApiKeyHandler(request)),
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
  options?: UseMutationOptions<{ message: string }, ErrorType<unknown>, string>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (keyId: string) => unwrapResponse<{ message: string }>(revokeApiKeyHandler(keyId)),
    onSuccess: () => {
      // Invalidate API keys list to refresh
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.apiKeys() })
    },
    ...options,
  })
}
