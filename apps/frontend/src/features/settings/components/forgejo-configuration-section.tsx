import { Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useDisconnectForgejo,
  useForgejoConfig,
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

/** Strips a scheme and any path, so a pasted URL and a bare host agree. */
function hostOf(value: string): string {
  const withoutScheme = value.trim().replace(/^https?:\/\//, '')
  return (withoutScheme.split('/')[0] ?? '').toLowerCase()
}

export function ForgejoConfigurationSection() {
  const [hostInput, setHostInput] = useState('')
  const [host, setHost] = useState('')
  const [pat, setPat] = useState('')

  const { data: config, isLoading: configLoading, error: configError } = useForgejoConfig(host)
  const disconnect = useDisconnectForgejo(host)
  const savePAT = useSaveForgejoPAT(host)
  const removePatOverride = useRemoveForgejoPatOverride(host)

  const handleLookup = () => {
    const next = hostOf(hostInput)
    if (!next) {
      toast.error('Please enter the Forgejo instance host')
      return
    }
    setHost(next)
  }

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
        },
        onError: (error) => {
          toast.error('Failed to save Forgejo connection', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleDisconnect = () => {
    disconnect.mutate(undefined, {
      onSuccess: () => {
        toast.success(`Disconnected from ${host}`)
      },
      onError: (error) => {
        toast.error('Failed to disconnect', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleRemovePatOverride = () => {
    removePatOverride.mutate(undefined, {
      onSuccess: () => {
        toast.success('Token removed, reverted to the OAuth credential')
      },
      onError: (error) => {
        toast.error('Failed to remove the token', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  // A full page load, not a router navigation: the flow leaves this origin for
  // the Forgejo instance and comes back through the API callback.
  const handleConnectOAuth = () => {
    const params = new URLSearchParams({
      host,
      return_to: window.location.href,
    })
    window.location.assign(`/api/v1/auth/forgejo/connect?${params.toString()}`)
  }

  const isConnected = config?.connected ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Forgejo Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect a Forgejo instance to access repositories and push changes. Each instance is
          connected separately.
        </p>

        <div className="space-y-4">
          <ConfigurationItem label="Instance" description="The host of your Forgejo instance">
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

          {host && configLoading && (
            <div className="flex items-center justify-center h-32">
              <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
            </div>
          )}

          {host && configError && (
            <Alert variant="destructive">
              <AlertTitle>Error</AlertTitle>
              <AlertDescription>
                Failed to load settings for {host}. Please try again later.
              </AlertDescription>
            </Alert>
          )}

          {config && (
            <>
              {isConnected && (
                <Alert>
                  <Plug className="h-4 w-4" />
                  <AlertTitle>Connected to {config.host}</AlertTitle>
                  <AlertDescription className="space-y-2">
                    <p className="text-sm">
                      {config.authMethod === 'forgejo_oauth'
                        ? 'Connected through the instance OAuth application.'
                        : 'Connected with a personal access token.'}
                    </p>
                    {config.hasPatOverride && (
                      <p className="text-sm">
                        This token is standing in front of an OAuth credential.
                      </p>
                    )}
                  </AlertDescription>
                </Alert>
              )}

              {config.oauthAvailable && !isConnected && (
                <ConfigurationItem
                  label="Sign in with Forgejo"
                  description="Authorize this deployment against the instance"
                >
                  <Button onClick={handleConnectOAuth} variant="outline">
                    Connect
                  </Button>
                </ConfigurationItem>
              )}

              {!config.oauthAvailable && (
                <Alert>
                  <AlertTitle>No OAuth application registered</AlertTitle>
                  <AlertDescription>
                    An administrator has not registered an OAuth application for {config.host}, so a
                    personal access token is the only way to connect it.
                  </AlertDescription>
                </Alert>
              )}

              <ConfigurationItem
                label="Personal Access Token"
                description="Needs the read:repository scope to clone, and write:repository to push"
              >
                <div className="flex gap-2">
                  <Input
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
                      Select the <span className="font-mono">repository</span> scope — Forgejo
                      requires at least one
                    </li>
                    <li>Generate the token and copy it</li>
                  </ol>
                </AlertDescription>
              </Alert>

              <div className="flex justify-end gap-2 pt-4">
                {config.hasPatOverride && (
                  <Button
                    onClick={handleRemovePatOverride}
                    disabled={removePatOverride.isPending}
                    variant="outline"
                    size="sm"
                  >
                    {removePatOverride.isPending && (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    )}
                    Use OAuth instead
                  </Button>
                )}
                {isConnected && (
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
            </>
          )}
        </div>
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
