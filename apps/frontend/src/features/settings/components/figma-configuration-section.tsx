import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  useDisconnectFigma,
  useFigmaConfig,
  useGetFigmaOAuthUrl,
  useSaveFigmaPAT,
} from '@/features/settings'
import { queryKeys } from '@/lib/api/query-keys'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

export function FigmaConfigurationSection() {
  const { user } = useAuth()
  const { data: figmaConfig, isLoading: configLoading, error: configError } = useFigmaConfig()
  const disconnectFigma = useDisconnectFigma()
  const savePAT = useSaveFigmaPAT()
  const [isConnecting, setIsConnecting] = useState(false)
  const [pat, setPat] = useState('')
  const queryClient = useQueryClient()

  // Fetch OAuth URL only when user clicks connect button
  const { refetch: fetchOAuthUrl } = useGetFigmaOAuthUrl({
    enabled: false,
  })

  // Listen for window focus to detect when OAuth popup closes
  // biome-ignore lint/correctness/useExhaustiveDependencies: queryClient is stable
  useEffect(() => {
    if (!isConnecting) {
      return
    }

    // When OAuth popup is opened, listen for window focus to detect when user returns
    const handleFocus = () => {
      // Invalidate Figma config to refetch connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.figma() })
    }

    window.addEventListener('focus', handleFocus)

    return () => {
      window.removeEventListener('focus', handleFocus)
    }
  }, [isConnecting, queryClient])

  // Reset connecting state and show success when connection is detected
  useEffect(() => {
    // When config updates and shows connected, reset connecting state
    if (isConnecting && figmaConfig?.connected) {
      setIsConnecting(false)
      toast.success('Successfully connected to Figma!')
    }
  }, [isConnecting, figmaConfig?.connected])

  // Handle connect button click
  const handleConnect = async () => {
    setIsConnecting(true)
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
          'figma-oauth',
          `width=${width},height=${height},left=${left},top=${top}`,
        )

        toast.info('Continue in the new window to connect your Figma account')
        // Don't set isConnecting=false yet - keep it true while OAuth is in progress
        // The focus event will trigger refetch, and we'll set it to false when connection succeeds
      } else {
        setIsConnecting(false)
      }
    } catch (error) {
      toast.error('Failed to initiate Figma OAuth', {
        description: getErrorMessage(error),
      })
      setIsConnecting(false)
    }
  }

  // Handle disconnect button click
  const handleDisconnect = () => {
    disconnectFigma.mutate(undefined, {
      onSuccess: () => {
        toast.success('Figma account disconnected')
        setPat('') // Clear PAT input
      },
      onError: (error) => {
        toast.error('Failed to disconnect Figma', {
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

  if (configError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load Figma settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  // If user is not authenticated, show warning
  if (!user) {
    return (
      <ConfigurationSectionWrapper>
        <ConfigurationGroup title="Figma Configuration" showSeparator={false}>
          <Alert>
            <AlertTitle>Authentication Required</AlertTitle>
            <AlertDescription>
              <p className="text-sm">
                You must be logged in to connect your Figma account. Please log in and try again.
              </p>
            </AlertDescription>
          </Alert>
        </ConfigurationGroup>
      </ConfigurationSectionWrapper>
    )
  }

  // Show connection status
  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Figma Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect your Figma account using OAuth or a Personal Access Token to enable design imports
        </p>

        {figmaConfig?.connected ? (
          <>
            {/* Connected status */}
            <Alert>
              <AlertTitle>Connected to Figma</AlertTitle>
              <AlertDescription>
                <div className="space-y-2">
                  <p className="text-sm">Your Figma account is connected and ready to use.</p>
                  <p className="text-sm">
                    <strong>Connection Type:</strong>{' '}
                    {figmaConfig.authMethod === 'oauth' ? 'OAuth' : 'Personal Access Token'}
                  </p>
                  {figmaConfig.figmaUserId && (
                    <p className="text-sm">
                      <strong>Figma User ID:</strong> {figmaConfig.figmaUserId}
                    </p>
                  )}
                  {figmaConfig.connectedAt && (
                    <p className="text-sm">
                      <strong>Connected:</strong>{' '}
                      {new Date(figmaConfig.connectedAt).toLocaleDateString()}
                    </p>
                  )}
                  {figmaConfig.expiresAt && (
                    <p className="text-sm">
                      <strong>Expires:</strong>{' '}
                      {new Date(figmaConfig.expiresAt).toLocaleDateString()}
                    </p>
                  )}
                </div>
              </AlertDescription>
            </Alert>

            {/* Disconnect button */}
            <div className="flex justify-end pt-4">
              <Button
                variant="destructive"
                size="sm"
                onClick={handleDisconnect}
                disabled={disconnectFigma.isPending}
              >
                {disconnectFigma.isPending ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <LogOut className="mr-2 h-4 w-4" />
                )}
                Disconnect Figma
              </Button>
            </div>
          </>
        ) : (
          <>
            {/* Personal Access Token Section */}
            <div className="space-y-4">
              <ConfigurationItem
                label="Personal Access Token"
                description="Provide your Figma Personal Access Token for API access"
              >
                <div className="flex gap-2">
                  <Input
                    type="password"
                    placeholder="Enter your Figma PAT"
                    value={pat}
                    onChange={(e) => setPat(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        handleSavePAT()
                      }
                    }}
                  />
                  <Button
                    onClick={handleSavePAT}
                    disabled={savePAT.isPending || !pat.trim()}
                    size="sm"
                  >
                    {savePAT.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                    Save Token
                  </Button>
                </div>
              </ConfigurationItem>

              {/* PAT Instructions Alert */}
              <Alert>
                <AlertTitle>How to generate a Personal Access Token</AlertTitle>
                <AlertDescription className="space-y-2">
                  <ol className="list-decimal list-inside space-y-1 text-sm">
                    <li>Login to your Figma account</li>
                    <li>
                      From the file browser, click the account menu in the top-left corner and
                      select <strong>Settings</strong>
                    </li>
                    <li>
                      Select the <strong>Security</strong> tab
                    </li>
                    <li>
                      In the <strong>Personal access tokens</strong> section, click{' '}
                      <strong>Generate new token</strong>
                    </li>
                    <li>Set the expiration and scopes for the token</li>
                    <li>
                      Required scopes: <code className="text-xs">file_content:read</code>,{' '}
                      <code className="text-xs">file_metadata:read</code>
                    </li>
                    <li>
                      Click <strong>Generate token</strong> and copy it (you won't be able to see it
                      again)
                    </li>
                    <li>Paste the token in the field above and click Save Token</li>
                  </ol>
                  <p className="text-sm text-muted-foreground mt-2">
                    For more information, visit{' '}
                    <a
                      href="https://www.figma.com/developers/api#access-tokens"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-600 hover:underline inline-flex items-center gap-1"
                    >
                      Figma Access Tokens Documentation
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </p>
                </AlertDescription>
              </Alert>

              {/* OAuth Section - Only show if OAuth is configured */}
              <ConfigurationItem
                label="OAuth Connection"
                description="Alternatively, connect using OAuth (if configured by administrator)"
              >
                <Button onClick={handleConnect} disabled={isConnecting} size="sm">
                  {isConnecting ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <Plug className="mr-2 h-4 w-4" />
                  )}
                  Connect via OAuth
                </Button>
              </ConfigurationItem>

              {/* OAuth Info Alert */}
              <Alert>
                <AlertTitle>OAuth Connection</AlertTitle>
                <AlertDescription className="space-y-2">
                  <p className="text-sm">
                    If your administrator has configured OAuth, you can connect using the button
                    above. The OAuth flow will open in a popup window where you can authorize the
                    application.
                  </p>
                  <p className="text-sm text-muted-foreground">
                    To configure OAuth, administrators should visit{' '}
                    <a
                      href="https://www.figma.com/developers/apps"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-600 hover:underline inline-flex items-center gap-1"
                    >
                      Figma Developer Apps
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </p>
                </AlertDescription>
              </Alert>
            </div>
          </>
        )}
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
