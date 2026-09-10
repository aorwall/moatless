import type { ForgejoProviderTokenStatusResponse } from '@moatless/api-client/sandbox/model'
import { Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useDisconnectForgejo,
  useForgejoConfig,
  useForgejoInstances,
  useRemoveForgejoPatOverride,
  useSaveForgejoPAT,
} from '@/features/settings/hooks/use-forgejo-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * The authority of what someone typed, so a pasted URL and a bare host agree.
 *
 * Matches the server's `normalize_host`: a host the two disagree on is a host
 * listed twice.
 */
function hostOf(value: string): string {
  const withoutScheme = value.trim().replace(/^https?:\/\//, '')
  const authority = withoutScheme.split(/[/?#]/)[0] ?? ''
  return authority.replace(/\.+$/, '').toLowerCase()
}

function connectionSummary(status: ForgejoProviderTokenStatusResponse): string {
  if (status.connected) {
    return status.authMethod === 'forgejo_oauth'
      ? 'Connected through the instance OAuth application.'
      : 'Connected with a personal access token.'
  }
  return status.oauthAvailable
    ? 'Not connected. Sign in to authorize this deployment.'
    : 'Not connected. This instance has no OAuth application, so a personal access token is the only way to connect it.'
}

export function ForgejoConfigurationSection() {
  const { data, isLoading, error } = useForgejoInstances()
  const instances = data?.instances ?? []

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Forgejo Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground">
          Connect a Forgejo instance to access repositories and push changes. Each instance is
          connected separately.
        </p>

        <div className="space-y-4">
          {isLoading && (
            <div className="flex items-center justify-center h-32">
              <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
            </div>
          )}

          {error && (
            <Alert variant="destructive">
              <AlertTitle>Error</AlertTitle>
              <AlertDescription>
                Failed to load the Forgejo instances. Please try again later.
              </AlertDescription>
            </Alert>
          )}

          {!(isLoading || error) && instances.length === 0 && (
            <Alert>
              <AlertTitle>No Forgejo instance registered</AlertTitle>
              <AlertDescription>
                An administrator registers an OAuth application for each instance. Name a host below
                to connect one with a personal access token.
              </AlertDescription>
            </Alert>
          )}

          {instances.map((instance) => (
            <ForgejoInstanceCard key={instance.host} status={instance} />
          ))}

          <OtherForgejoInstance knownHosts={instances.map((instance) => instance.host)} />
        </div>
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}

/** One instance, with the caller's connection to it and every action on it. */
function ForgejoInstanceCard({ status }: { status: ForgejoProviderTokenStatusResponse }) {
  const host = status.host
  const disconnect = useDisconnectForgejo(host)
  const removePatOverride = useRemoveForgejoPatOverride(host)
  const [tokenFormOpen, setTokenFormOpen] = useState(false)

  // A full page load, not a router navigation: the flow leaves this origin for
  // the Forgejo instance and comes back through the API callback.
  const handleConnectOAuth = () => {
    const params = new URLSearchParams({
      host,
      return_to: window.location.href,
    })
    window.location.assign(`/api/v1/auth/forgejo/connect?${params.toString()}`)
  }

  const handleDisconnect = () => {
    disconnect.mutate(undefined, {
      onSuccess: () => {
        toast.success(`Disconnected from ${host}`)
      },
      onError: (error) => {
        toast.error('Failed to disconnect', { description: getErrorMessage(error) })
      },
    })
  }

  const handleRemovePatOverride = () => {
    removePatOverride.mutate(undefined, {
      onSuccess: () => {
        toast.success('Token removed, reverted to the OAuth credential')
      },
      onError: (error) => {
        toast.error('Failed to remove the token', { description: getErrorMessage(error) })
      },
    })
  }

  const showTokenForm = tokenFormOpen || !(status.connected || status.oauthAvailable)

  return (
    <div className="rounded-lg border p-4 space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <p className="text-sm font-medium font-mono">{host}</p>
          <p className="text-sm text-muted-foreground">{connectionSummary(status)}</p>
          {status.hasPatOverride && (
            <p className="text-sm text-muted-foreground">
              This token is standing in front of an OAuth credential.
            </p>
          )}
        </div>

        {status.oauthAvailable && !status.connected && (
          <Button onClick={handleConnectOAuth} variant="outline" size="sm">
            <Plug className="mr-2 h-4 w-4" />
            Connect
          </Button>
        )}
      </div>

      {showTokenForm ? (
        <ForgejoTokenForm host={host} onSaved={() => setTokenFormOpen(false)} />
      ) : (
        <Button
          onClick={() => setTokenFormOpen(true)}
          variant="ghost"
          size="sm"
          className="px-0 text-muted-foreground"
        >
          {status.connected
            ? 'Replace with a personal access token'
            : 'Use a personal access token'}
        </Button>
      )}

      {(status.connected || status.hasPatOverride) && (
        <div className="flex justify-end gap-2">
          {status.hasPatOverride && (
            <Button
              onClick={handleRemovePatOverride}
              disabled={removePatOverride.isPending}
              variant="outline"
              size="sm"
            >
              {removePatOverride.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Use OAuth instead
            </Button>
          )}
          {status.connected && (
            <Button
              onClick={handleDisconnect}
              disabled={disconnect.isPending}
              variant="outline"
              size="sm"
            >
              {disconnect.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="mr-2 h-4 w-4" />
              )}
              Disconnect
            </Button>
          )}
        </div>
      )}
    </div>
  )
}

function ForgejoTokenForm({ host, onSaved }: { host: string; onSaved: () => void }) {
  const [pat, setPat] = useState('')
  const savePAT = useSaveForgejoPAT()

  const handleSavePAT = () => {
    if (!pat.trim()) {
      toast.error('Please enter a Personal Access Token')
      return
    }

    savePAT.mutate(
      { baseUrl: host, token: pat },
      {
        onSuccess: () => {
          toast.success(`Connected to ${host}`)
          setPat('')
          onSaved()
        },
        onError: (error) => {
          toast.error('Failed to save Forgejo connection', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  return (
    <div className="space-y-4">
      <ConfigurationItem
        label="Personal Access Token"
        description="Needs the read:repository scope to clone, and write:repository to push"
        htmlFor={`forgejo-pat-${host}`}
      >
        <div className="flex gap-2">
          <Input
            id={`forgejo-pat-${host}`}
            type="password"
            placeholder="Enter your Forgejo personal access token"
            value={pat}
            onChange={(e) => setPat(e.target.value)}
            className="flex-1 font-mono"
          />
          <Button
            onClick={handleSavePAT}
            disabled={savePAT.isPending || !pat.trim()}
            variant="outline"
          >
            {savePAT.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Save'}
          </Button>
        </div>
      </ConfigurationItem>

      <Alert>
        <AlertTitle>How to create a Personal Access Token</AlertTitle>
        <AlertDescription className="space-y-2">
          <ol className="list-decimal list-inside space-y-1 text-sm">
            <li>Log in to your Forgejo instance</li>
            <li>
              Go to <strong>Settings</strong> &rarr; <strong>Applications</strong>
            </li>
            <li>
              Under <strong>Manage Access Tokens</strong>, give the token a name
            </li>
            <li>
              Select the <span className="font-mono">repository</span> scope — Forgejo requires at
              least one
            </li>
            <li>Generate the token and copy it</li>
          </ol>
        </AlertDescription>
      </Alert>
    </div>
  )
}

/**
 * An instance the list does not carry.
 *
 * `GET /settings/forgejo/instances` returns the hosts an administrator
 * registered an OAuth application for, and a personal access token connects a
 * host without one.
 */
function OtherForgejoInstance({ knownHosts }: { knownHosts: string[] }) {
  const [hostInput, setHostInput] = useState('')
  const [host, setHost] = useState('')
  const { data: config, isLoading, error } = useForgejoConfig(host)

  const handleLookup = () => {
    const next = hostOf(hostInput)
    if (!next) {
      toast.error('Please enter the Forgejo instance host')
      return
    }
    if (knownHosts.includes(next)) {
      toast.info(`${next} is listed above`)
      setHost('')
      return
    }
    setHost(next)
  }

  return (
    <div className="space-y-4 pt-2">
      <ConfigurationItem
        label="Connect another instance"
        description="The host of a Forgejo instance that is not listed above"
      >
        <div className="flex gap-2">
          <Input
            type="text"
            placeholder="git.example.com"
            value={hostInput}
            onChange={(e) => setHostInput(e.target.value)}
            className="flex-1 font-mono"
          />
          <Button onClick={handleLookup} disabled={!hostInput.trim()} variant="outline">
            Check
          </Button>
        </div>
      </ConfigurationItem>

      {host && isLoading && (
        <div className="flex items-center justify-center h-32">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      )}

      {host && error && (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>
            Failed to load settings for {host}. Please try again later.
          </AlertDescription>
        </Alert>
      )}

      {config && <ForgejoInstanceCard status={config} />}
    </div>
  )
}
