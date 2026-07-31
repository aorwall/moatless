import type { CreatePairingCodeRequest, PairingCodeResponse } from '@moatless/api-client'
import type { ErrorType } from '@moatless/api-client/custom-instance'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { createPairingCodeHandler } from '@moatless/api-client/sandbox/auth/auth'
import { type UseMutationOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

export type { CreatePairingCodeRequest, PairingCodeResponse } from '@moatless/api-client'

/**
 * Mint a code that pairs a client on another device to this account.
 *
 * There is no list and no read: the code is in this response and nowhere else,
 * and a lost one is replaced rather than recovered. Redeeming it mints an API
 * key, so the keys list is invalidated — that list is where a paired device
 * shows up and where it is revoked.
 */
export function usePairingCode(
  options?: UseMutationOptions<PairingCodeResponse, ErrorType<unknown>, CreatePairingCodeRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreatePairingCodeRequest) =>
      unwrapResponse<PairingCodeResponse>(createPairingCodeHandler(request)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.apiKeys() })
    },
    ...options,
  })
}
