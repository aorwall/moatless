import type {
  ListSecretsParams,
  PatchSecretRequest,
  PutSecretRequest,
  SecretMetadataResponse,
  SecretMutationResponse,
} from '@moatless/api-client'
import { type ErrorType, unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteSecret,
  listSecrets,
  patchSecret,
  putSecret,
} from '@moatless/api-client/sandbox/secrets/secrets'
import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

export type { PatchSecretRequest, PutSecretRequest, SecretMetadataResponse }

/**
 * List secret metadata for a given scope. Values are never returned by the API.
 */
export function useSecretsList(params: ListSecretsParams, options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.secrets.list({ scope: params.scope, scopeId: params.scopeId }),
    queryFn: () => unwrapResponse<SecretMetadataResponse[]>(listSecrets(params)),
    enabled: options?.enabled ?? true,
  })
}

/**
 * Create or replace a secret value. Backend replaces on (scope, scope_id, key) conflict.
 */
export function usePutSecret(
  options?: UseMutationOptions<SecretMutationResponse, ErrorType<unknown>, PutSecretRequest>,
) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: PutSecretRequest) =>
      unwrapResponse<SecretMutationResponse>(putSecret(request)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.secrets.all() })
    },
    ...options,
  })
}

/**
 * Patch a secret's mutable fields (value/kind). Key and scope are immutable.
 */
export function usePatchSecret(
  options?: UseMutationOptions<
    SecretMutationResponse,
    ErrorType<unknown>,
    { id: string; request: PatchSecretRequest }
  >,
) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: PatchSecretRequest }) =>
      unwrapResponse<SecretMutationResponse>(patchSecret(id, request)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.secrets.all() })
    },
    ...options,
  })
}

/**
 * Delete a secret by id.
 */
export function useDeleteSecret(
  options?: UseMutationOptions<SecretMutationResponse, ErrorType<unknown>, string>,
) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => unwrapResponse<SecretMutationResponse>(deleteSecret(id)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.secrets.all() })
    },
    ...options,
  })
}
