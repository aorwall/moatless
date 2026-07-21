import { CheckCircle2, Loader2, Plug } from 'lucide-react'
import { toast } from 'sonner'

import { GitHubPatForm } from '@/features/settings/components/github-pat-form'
import { GitHubPatHelp } from '@/features/settings/components/github-pat-help'
import {
  useAdminGitHubConfig,
  useRemoveAdminGitHubPAT,
  useSaveAdminGitHubPAT,
} from '@/features/settings/hooks/use-admin-github-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * Org-wide GitHub token. Sits at the bottom of the `user > team > global`
 * precedence chain, so it only applies to users who have no token of their own.
 */
export function AdminGitHubIntegrationSection() {
  const { data: config, isLoading, error } = useAdminGitHubConfig()
  const savePAT = useSaveAdminGitHubPAT()
  const removePAT = useRemoveAdminGitHubPAT()

  const handleSave = async (token: string) => {
    const result = await savePAT.mutateAsync({ token })
    toast.success('Organization GitHub token saved', {
      description: result.login ? `Authenticated as ${result.login}` : undefined,
    })
    return result
  }

  const handleRemove = () => {
    removePAT.mutate(undefined, {
      onSuccess: () => toast.success('Organization GitHub token removed'),
      onError: (e) =>
        toast.error('Failed to remove the organization GitHub token', {
          description: getErrorMessage(e),
        }),
    })
  }

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        Loading GitHub configuration…
      </div>
    )
  }

  const isConnected = config?.connected ?? false

  return (
    <ConfigurationSectionWrapper className="p-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">GitHub</h2>
        <p className="text-sm text-muted-foreground">
          An organization-wide GitHub token, used by any user who has not connected GitHub
          themselves. A user&apos;s own token always takes precedence.
        </p>
      </div>

      <ConfigurationGroup showSeparator={false}>
        {error && <ErrorAlert error={error} />}

        {isConnected && (
          <Alert>
            <CheckCircle2 className="h-4 w-4" />
            <AlertTitle>Organization token configured</AlertTitle>
            <AlertDescription>
              Users without their own GitHub connection will use this token to read repositories.
              {config?.login && (
                <>
                  {' '}
                  <strong>Authenticated as:</strong> {config.login}
                </>
              )}
            </AlertDescription>
          </Alert>
        )}

        {!isConnected && (
          <Alert>
            <Plug className="h-4 w-4" />
            <AlertTitle>No organization token set</AlertTitle>
            <AlertDescription>
              Every user must connect GitHub individually until an organization token is set.
            </AlertDescription>
          </Alert>
        )}

        <ConfigurationItem
          label="Personal Access Token"
          description="Validated against GitHub before it is stored. The value is encrypted at rest and never returned by the API."
        >
          <GitHubPatForm
            hasToken={isConnected}
            onSave={handleSave}
            isSaving={savePAT.isPending}
            onRemove={handleRemove}
            isRemoving={removePAT.isPending}
            removeLabel="Remove organization token"
          />
        </ConfigurationItem>

        {savePAT.error && <ErrorAlert error={savePAT.error} />}

        {!isConnected && (
          <GitHubPatHelp scopeHint="Select the &quot;repo&quot; scope, or grant fine-grained read access to your organization's repositories" />
        )}
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
