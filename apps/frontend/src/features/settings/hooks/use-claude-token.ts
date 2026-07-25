import { Scope, SecretKind } from '@moatless/api-client'
import type { ErrorType } from '@moatless/api-client/custom-instance'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { useDeleteSecret, usePutSecret, useSecretsList } from '@/features/secrets/hooks/use-secrets'
import { getErrorMessage } from '@/lib/utils/error'

/** Env var the sandbox reads to authenticate Claude Code. */
export const CLAUDE_TOKEN_SECRET_KEY = 'CLAUDE_CODE_OAUTH_TOKEN'

function mutationErrorMessage(error: ErrorType<unknown>): string {
  // A 503 here means the server has no encryption key configured; the raw
  // message would just say "Service Unavailable".
  return error.status === 503
    ? 'Secret encryption is not configured on this server.'
    : getErrorMessage(error)
}

/**
 * Reads and writes the Claude token as a user-scoped secret — the same storage
 * as Settings → Secrets, so both views stay in sync.
 */
export function useClaudeToken() {
  const { user, isLoading: authLoading } = useAuth()
  const userId = user?.id

  // Listed with the user id (writes send null) so this shares a cache entry
  // with the secrets page: `queryKeys.secrets.list` embeds the params.
  const secretsQuery = useSecretsList({ scope: Scope.user }, { enabled: !!userId })

  // A disabled row reads as not configured; saving re-enables it.
  const claudeSecret =
    secretsQuery.data?.find((s) => s.key === CLAUDE_TOKEN_SECRET_KEY && s.enabled) ?? null

  // `onError` belongs on the hook, not on `mutate()`: the global MutationCache
  // only suppresses its fallback toast when `mutation.options.onError` is set.
  // Success callbacks stay at the call site, or they would replace the
  // cache-invalidating `onSuccess` in `usePutSecret`/`useDeleteSecret`.
  const putSecret = usePutSecret({
    onError: (error) => {
      toast.error('Failed to save Claude token', { description: mutationErrorMessage(error) })
    },
  })

  const deleteSecret = useDeleteSecret({
    onError: (error) => {
      toast.error('Failed to remove Claude token', { description: mutationErrorMessage(error) })
    },
  })

  const saveToken = (token: string, callbacks?: { onSuccess?: () => void }) => {
    putSecret.mutate(
      {
        scope: Scope.user,
        key: CLAUDE_TOKEN_SECRET_KEY,
        kind: SecretKind.env,
        value: token,
      },
      callbacks,
    )
  }

  const removeToken = (callbacks?: { onSuccess?: () => void }) => {
    if (!claudeSecret) {
      return
    }
    deleteSecret.mutate(claudeSecret.id, callbacks)
  }

  return {
    claudeSecret,
    // Without the auth guard the disconnected form flashes before the session
    // resolves: a disabled query reports `isLoading: false`.
    isLoading: authLoading || secretsQuery.isLoading,
    error: secretsQuery.error,
    saveToken,
    removeToken,
    isSaving: putSecret.isPending,
    isRemoving: deleteSecret.isPending,
  }
}
