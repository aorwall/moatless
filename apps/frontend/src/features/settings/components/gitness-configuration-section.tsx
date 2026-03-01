import { Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useDisconnectGitness,
  useGitnessConfig,
  useSaveGitnessPAT,
} from '@/features/settings/hooks/use-gitness-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

export function GitnessConfigurationSection() {
  const { data: gitnessConfig, isLoading: configLoading, error: gitnessError } = useGitnessConfig()
  const disconnectGitness = useDisconnectGitness()
  const savePAT = useSaveGitnessPAT()
  const [pat, setPat] = useState('')
  const [baseUrl, setBaseUrl] = useState('')

  const handleDisconnect = () => {
    disconnectGitness.mutate(undefined, {
      onSuccess: () => {
        toast.success('Disconnected from Gitness')
      },
      onError: (error) => {
        toast.error('Failed to disconnect', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleSavePAT = () => {
    if (!baseUrl.trim()) {
      toast.error('Please enter the Gitness server URL')
      return
    }

    if (!pat.trim()) {
      toast.error('Please enter a Personal Access Token')
      return
    }

    savePAT.mutate(
      { baseUrl: baseUrl.trim(), token: pat },
      {
        onSuccess: () => {
          toast.success('Gitness connection saved successfully')
          setPat('')
          setBaseUrl('')
        },
        onError: (error) => {
          toast.error('Failed to save Gitness connection', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  if (configLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (gitnessError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>
          Failed to load Gitness settings. Please try again later.
        </AlertDescription>
      </Alert>
    )
  }

  const isConnected = gitnessConfig?.connected ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Gitness Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect your Harness Gitness instance to access repositories and push changes
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to Gitness</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">
                Your Gitness account is connected via Personal Access Token.
              </p>
              {gitnessConfig?.login && (
                <p className="text-sm">
                  <strong>Username:</strong> {gitnessConfig.login}
                </p>
              )}
              {gitnessConfig?.baseUrl && (
                <p className="text-sm">
                  <strong>Server:</strong> {gitnessConfig.baseUrl}
                </p>
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* PAT Input (when not connected) */}
        {!isConnected && (
          <div className="space-y-4">
            <ConfigurationItem
              label="Server URL"
              description="The base URL of your Gitness instance"
            >
              <Input
                type="url"
                placeholder="https://gitness.example.com"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                className="font-mono"
              />
            </ConfigurationItem>

            <ConfigurationItem
              label="Personal Access Token"
              description="Use a Gitness PAT for authentication"
            >
              <div className="flex gap-2">
                <Input
                  type="password"
                  placeholder="Enter your Gitness personal access token"
                  value={pat}
                  onChange={(e) => setPat(e.target.value)}
                  className="flex-1 font-mono"
                />
                <Button
                  onClick={handleSavePAT}
                  disabled={savePAT.isPending || !pat.trim() || !baseUrl.trim()}
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
                  <li>Log in to your Gitness instance</li>
                  <li>
                    Go to your <span className="font-medium">User Settings</span> (click your
                    avatar, then "My Account")
                  </li>
                  <li>
                    Navigate to <strong>Tokens</strong> and click "New Token"
                  </li>
                  <li>Give your token a descriptive name and set an expiration</li>
                  <li>Copy the generated token</li>
                </ol>
              </AlertDescription>
            </Alert>
          </div>
        )}

        {/* Disconnect Button (when connected) */}
        {isConnected && (
          <div className="flex justify-end pt-4">
            <Button
              onClick={handleDisconnect}
              disabled={disconnectGitness.isPending}
              variant="outline"
              size="sm"
            >
              {disconnectGitness.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="mr-2 h-4 w-4" />
              )}
              Disconnect
            </Button>
          </div>
        )}
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
