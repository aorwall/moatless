import type {
  GitHubAppOption,
  GitHubInstallationBindingResponse,
} from '@moatless/api-client/sandbox/model'
import { CheckCircle2, ExternalLink, Loader2, Plug, RefreshCw, Unplug } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'

import {
  useAdminGitHubApps,
  useAdminGitHubInstallation,
  useRemoveAdminGitHubInstallation,
  useSaveAdminGitHubInstallation,
} from '@/features/settings/hooks/use-admin-github-connection'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { getErrorMessage } from '@/lib/utils/error'

interface GitHubInstallationCardProps {
  teamId?: string
}

function InstallationStatus({
  binding,
}: {
  binding: GitHubInstallationBindingResponse | undefined
}) {
  if (!binding?.configured) {
    return (
      <Alert>
        <Plug className="h-4 w-4" />
        <AlertTitle>No installation bound</AlertTitle>
        <AlertDescription>
          GitHub repositories in this scope cannot authenticate until an installation is bound.
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <Alert>
      <CheckCircle2 className="h-4 w-4" />
      <AlertTitle>Installation configured</AlertTitle>
      <AlertDescription>
        <strong>{binding.accountLogin}</strong>
        {binding.accountType ? ` (${binding.accountType})` : ''} via App{' '}
        <strong>{binding.githubAppKey}</strong>, installation {binding.installationId}.
      </AlertDescription>
    </Alert>
  )
}

function InstallAppLink({ app }: { app: GitHubAppOption | undefined }) {
  if (!app) {
    return null
  }

  return (
    <Button asChild type="button" variant="outline">
      <a href={app.installUrl} target="_blank" rel="noreferrer">
        <ExternalLink />
        Install App on GitHub
      </a>
    </Button>
  )
}

export function GitHubInstallationCard({ teamId }: GitHubInstallationCardProps) {
  const appsQuery = useAdminGitHubApps()
  const bindingQuery = useAdminGitHubInstallation(teamId)
  const saveBinding = useSaveAdminGitHubInstallation(teamId)
  const removeBinding = useRemoveAdminGitHubInstallation(teamId)
  const [appKey, setAppKey] = useState('')
  const [installationId, setInstallationId] = useState('')

  useEffect(() => {
    if (bindingQuery.data?.githubAppKey) {
      setAppKey(bindingQuery.data.githubAppKey)
    }
    if (bindingQuery.data?.installationId) {
      setInstallationId(String(bindingQuery.data.installationId))
    }
  }, [bindingQuery.data?.githubAppKey, bindingQuery.data?.installationId])

  const selectedApp = useMemo(
    () => appsQuery.data?.apps.find((app) => app.githubAppKey === appKey),
    [appKey, appsQuery.data?.apps],
  )

  const handleAppChange = (value: string) => {
    setAppKey(value)
    setInstallationId('')
  }

  const handleBind = () => {
    const parsedInstallationId = Number(installationId)
    if (!(appKey && Number.isSafeInteger(parsedInstallationId))) {
      return
    }
    saveBinding.mutate(
      { githubAppKey: appKey, installationId: parsedInstallationId },
      {
        onSuccess: (binding) =>
          toast.success('GitHub installation bound', {
            description: binding.accountLogin
              ? `Repositories will authenticate as ${binding.accountLogin}`
              : undefined,
          }),
        onError: (error) =>
          toast.error('Failed to bind GitHub installation', {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  const handleUnbind = () => {
    removeBinding.mutate(undefined, {
      onSuccess: () => {
        setInstallationId('')
        toast.success('GitHub installation unbound')
      },
      onError: (error) =>
        toast.error('Failed to unbind GitHub installation', {
          description: getErrorMessage(error),
        }),
    })
  }

  const isLoading = appsQuery.isLoading || bindingQuery.isLoading
  const binding = bindingQuery.data
  const title = teamId ? 'Team GitHub access' : 'Global GitHub access'

  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>
          Bind one configured GitHub App installation. Repositories in this scope must belong to the
          installation account; credentials from other scopes are never used.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading GitHub Apps…
          </div>
        ) : (
          <>
            {(appsQuery.error || bindingQuery.error) && (
              <ErrorAlert error={appsQuery.error ?? bindingQuery.error} />
            )}

            <InstallationStatus binding={binding} />

            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <label className="text-sm font-medium" htmlFor={`github-app-${teamId ?? 'global'}`}>
                  GitHub App
                </label>
                <Select value={appKey} onValueChange={handleAppChange}>
                  <SelectTrigger id={`github-app-${teamId ?? 'global'}`}>
                    <SelectValue placeholder="Select an App" />
                  </SelectTrigger>
                  <SelectContent>
                    {appsQuery.data?.apps.map((app) => (
                      <SelectItem key={app.githubAppKey} value={app.githubAppKey}>
                        {app.name} ({app.githubAppKey})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <label
                  className="text-sm font-medium"
                  htmlFor={`github-installation-${teamId ?? 'global'}`}
                >
                  Installation
                </label>
                <Select
                  value={installationId}
                  onValueChange={setInstallationId}
                  disabled={!selectedApp}
                >
                  <SelectTrigger id={`github-installation-${teamId ?? 'global'}`}>
                    <SelectValue placeholder="Select an installation" />
                  </SelectTrigger>
                  <SelectContent>
                    {selectedApp?.installations.map((installation) => (
                      <SelectItem
                        key={installation.installationId}
                        value={String(installation.installationId)}
                      >
                        {installation.accountLogin}
                        {installation.accountType ? ` (${installation.accountType})` : ''}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                onClick={handleBind}
                disabled={saveBinding.isPending || !(appKey && installationId)}
              >
                {saveBinding.isPending && <Loader2 className="animate-spin" />}
                Bind installation
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => appsQuery.refetch()}
                disabled={appsQuery.isFetching}
              >
                <RefreshCw className={appsQuery.isFetching ? 'animate-spin' : undefined} />
                Refresh installations
              </Button>
              <InstallAppLink app={selectedApp} />
              {binding?.configured && (
                <Button
                  type="button"
                  variant="destructive"
                  onClick={handleUnbind}
                  disabled={removeBinding.isPending}
                >
                  <Unplug />
                  Unbind
                </Button>
              )}
            </div>

            <p className="text-xs text-muted-foreground">
              Unbinding removes only the Moatless binding. It does not uninstall the App from
              GitHub.
            </p>
          </>
        )}
      </CardContent>
    </Card>
  )
}
