import type { PasswordLoginRequest } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { passwordLoginHandler } from '@moatless/api-client/sandbox/auth/auth'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { consumeReturnTo } from '../utils/return-to'

/**
 * Log in with a username and password.
 *
 * `meta.skipAuthRedirect` is load-bearing: a wrong password answers 401, and
 * without the opt-out the global handler in `query-client.ts` would navigate
 * away from `/login` and burn the redirect cooldown.
 */
export function usePasswordLogin() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: PasswordLoginRequest) => unwrapResponse(passwordLoginHandler(request)),
    meta: { skipAuthRedirect: true },
    onSuccess: () => {
      // Drop every cached query before navigating. Without this a second login
      // in the same browser context renders the previous user's cached data.
      queryClient.clear()
      window.location.replace(consumeReturnTo() ?? '/')
    },
  })
}
