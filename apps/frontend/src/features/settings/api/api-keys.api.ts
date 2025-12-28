import { buildPath, type RouteRequestJson, type RouteSuccess } from '@moatless/api-contract'
import {
  createApiKeyRoute,
  listApiKeysRoute,
  revokeApiKeyRoute,
} from '@moatless/api-contract/api-keys'
import { apiClient } from '@/lib/api/client'

type ListApiKeysResponse = RouteSuccess<typeof listApiKeysRoute>
type CreateApiKeyRequest = RouteRequestJson<typeof createApiKeyRoute>
type CreateApiKeyResponse = RouteSuccess<typeof createApiKeyRoute>
type RevokeApiKeyResponse = RouteSuccess<typeof revokeApiKeyRoute>

export const apiKeysApi = {
  /**
   * List all API keys for the current user
   */
  list: async (): Promise<ListApiKeysResponse> => {
    const path = buildPath(listApiKeysRoute)
    return await apiClient.get<ListApiKeysResponse>(path)
  },

  /**
   * Create a new API key
   * Note: The full key is only returned once and cannot be retrieved later
   */
  create: async (request: CreateApiKeyRequest): Promise<CreateApiKeyResponse> => {
    const path = buildPath(createApiKeyRoute)
    return await apiClient.post<CreateApiKeyResponse>(path, request)
  },

  /**
   * Revoke an API key
   */
  revoke: async (keyId: string): Promise<RevokeApiKeyResponse> => {
    // Build path and replace the path parameter
    const basePath = buildPath(revokeApiKeyRoute)
    const path = basePath.replace('{keyId}', keyId)
    return await apiClient.delete<RevokeApiKeyResponse>(path)
  },
}
