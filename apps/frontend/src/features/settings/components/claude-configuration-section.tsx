import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useClaudeConfig,
  useDisconnectClaude,
  useGetClaudeOAuthUrl,
  useSaveClaudePAT,
} from '@/features/settings'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

export function ClaudeConfigurationSection() {
  const { data: claudeConfig, isLoading: configLoading, error: claudeError } = useClaudeConfig()
  const disconnectClaude = useDisconnectClaude()
  const savePAT = useSaveClaudePAT()
  const [pat, setPat] = useState('')

  // Fetch OAuth URL only when user clicks connect button
  const { refetch: fetchOAuthUrl } = useGetClaudeOAuthUrl({
    enabled: false,
  })

  // Handle connect button click
  const handleConnect = async () => {
    try {
      const result = await fetchOAuthUrl()
      if (result.data?.authUrl) {
        // Open OAuth URL in new window
        const width = 600
        const height = 700
        const left = window.screen.width / 2 - width / 2
        const top = window.screen.height / 2 - height / 2

        window.open(
          result.data.authUrl,
          'claude-oauth',
          `width=${width},height=${height},left=${left},top=${top}`,
        )

        toast.info('Copy the Claude token from the popup page and paste it in the field above')
      }
    } catch (error) {
      toast.error('Failed to open OAuth popup', {
        description: getErrorMessage(error),
      })
    }
  }

  // Handle disconnect button click
  const handleDisconnect = () => {
    disconnectClaude.mutate(undefined, {
      onSuccess: () => {
        toast.success('Claude account disconnected')
        setPat('') // Clear PAT input
      },
      onError: (error) => {
        toast.error('Failed to disconnect Claude', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  // Handle save PAT button click
  const handleSavePAT = () => {
    if (!pat.trim()) {
      toast.error('Please enter a Personal Access Token')
      return
    }

    savePAT.mutate(
      { accessToken: pat },
      {
        onSuccess: () => {
          toast.success('Personal Access Token saved successfully')
          setPat('') // Clear input after successful save
        },
        onError: (error) => {
          toast.error('Failed to save Personal Access Token', {
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

  if (claudeError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load Claude settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  // Show service connection UI
  const isConnected = claudeConfig?.connected ?? false
  const hasOAuthEnabled = claudeConfig?.oauthEnabled ?? true

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Claude Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect your Claude account to enable Claude Code integration
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to Claude</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">
                Your Claude account is connected via{' '}
                {claudeConfig?.method === 'oauth'
                  ? 'OAuth'
                  : claudeConfig?.method === 'pat'
                    ? 'Personal Access Token'
                    : claudeConfig?.method === 'api_key'
                      ? 'API Key'
                      : 'file-based token'}
                .
              </p>
              {claudeConfig?.connectedAt && (
                <p className="text-sm text-muted-foreground">
                  Connected on {new Date(claudeConfig.connectedAt).toLocaleDateString()}
                </p>
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* Claude Token Input (shown when not connected) */}
        {!isConnected && (
          <div className="space-y-4">
            <ConfigurationItem
              label="Claude Token"
              description="Enter your Claude personal access token to connect your account"
            >
              <div className="space-y-3">
                <div className="flex gap-2">
                  <Input
                    type="password"
                    placeholder="Enter your Claude token"
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

                {/* OAuth button to help get token */}
                {hasOAuthEnabled && (
                  <Button
                    onClick={handleConnect}
                    variant="outline"
                    size="sm"
                    className="w-full sm:w-auto"
                  >
                    <ExternalLink className="mr-2 h-4 w-4" />
                    Get Token via OAuth Popup
                  </Button>
                )}
              </div>
            </ConfigurationItem>

            <Alert>
              <AlertTitle>How to get a Claude token</AlertTitle>
              <AlertDescription className="space-y-2">
                <p className="text-sm mb-2">You can get a token in two ways:</p>
                <ol className="list-decimal list-inside space-y-1 text-sm">
                  <li>
                    <span className="font-medium">Via OAuth popup:</span> Click the "Get Token via
                    OAuth Popup" button above, authorize in the popup, and copy the Claude token
                    shown on the page after authorization
                  </li>
                  <li>
                    <span className="font-medium">Via terminal:</span> Run{' '}
                    <code className="bg-muted px-1 py-0.5 rounded text-xs">claude setup-token</code>
                    , follow the authentication flow, and copy the token from the command output
                  </li>
                </ol>
                <p className="text-sm mt-2">
                  For more information, see the{' '}
                  <a
                    href="https://docs.anthropic.com/claude/docs/claude-code"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-600 hover:underline inline-flex items-center gap-1"
                  >
                    Claude Code documentation
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </p>
              </AlertDescription>
            </Alert>
          </div>
        )}

        {/* Disconnect Button (when connected) */}
        {isConnected && (
          <div className="flex justify-end pt-4">
            <Button
              onClick={handleDisconnect}
              disabled={disconnectClaude.isPending}
              variant="outline"
              size="sm"
            >
              {disconnectClaude.isPending ? (
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
