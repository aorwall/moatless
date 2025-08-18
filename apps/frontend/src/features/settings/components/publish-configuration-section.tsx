import type { UpdatePublishSettingsRequest } from '@moatless/types'
import { Eye, EyeOff, Loader2, RotateCcw, Save } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { usePublishSettings, useUpdatePublishSettings } from '@/features/settings'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

export function PublishConfigurationSection() {
  const {
    data: publishSettings,
    isLoading: publishLoading,
    error: publishError,
  } = usePublishSettings()
  const updatePublishSettings = useUpdatePublishSettings()

  const [showToken, setShowToken] = useState(false)

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const basePublishFormData = useMemo<UpdatePublishSettingsRequest>(
    () => ({
      netlifyAccessToken: publishSettings?.settings.netlifyAccessToken || null,
    }),
    [publishSettings],
  )

  // Track user modifications separately from the derived data
  const [publishUserModifications, setPublishUserModifications] = useState<
    Partial<UpdatePublishSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const publishFormData = useMemo(
    () => ({
      ...basePublishFormData,
      ...publishUserModifications,
    }),
    [basePublishFormData, publishUserModifications],
  )

  if (publishLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (publishError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>
          Failed to load Publish settings. Please try again later.
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Publish Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Configure publishing settings for deploying your applications
        </p>

        <ConfigurationItem
          label="Netlify Access Token"
          description="Personal access token for deploying to Netlify"
        >
          <div className="flex gap-2">
            <ConfigurationInput
              type={showToken ? 'text' : 'password'}
              placeholder={
                publishFormData.netlifyAccessToken
                  ? 'Token configured'
                  : 'Enter your Netlify access token'
              }
              value={publishFormData.netlifyAccessToken || ''}
              className="flex-1 font-mono"
              onChange={(e) => {
                const value = e.target.value.trim()
                setPublishUserModifications((prev) => ({
                  ...prev,
                  netlifyAccessToken: value === '' ? null : value,
                }))
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={() => setShowToken(!showToken)}
              title={showToken ? 'Hide token' : 'Show token'}
            >
              {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </Button>
          </div>
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Info Alert */}
      <Alert>
        <AlertTitle>How to create a Netlify Access Token</AlertTitle>
        <AlertDescription className="space-y-2">
          <ol className="list-decimal list-inside space-y-1 text-sm">
            <li>
              Go to{' '}
              <a
                href="https://app.netlify.com/user/applications#personal-access-tokens"
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-600 hover:underline"
              >
                Netlify → User Settings → Applications
              </a>
            </li>
            <li>Scroll down to "Personal access tokens"</li>
            <li>Click "New access token"</li>
            <li>Give your token a descriptive name</li>
            <li>Click "Generate token" and copy the token</li>
          </ol>
          <p className="text-sm text-muted-foreground mt-2">
            The token will be used to authenticate deployments to Netlify. Keep it secure and never
            share it.
          </p>
        </AlertDescription>
      </Alert>

      {/* Actions */}
      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between pt-4">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            // Reset Publish settings to defaults by clearing modifications
            setPublishUserModifications({})
            toast.success('Publish settings reset to defaults')
          }}
          disabled={false}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset to Defaults
        </Button>
        <Button
          onClick={() => {
            updatePublishSettings.mutate(publishFormData, {
              onSuccess: () => {
                setPublishUserModifications({})
                toast.success('Publish settings saved successfully')
              },
              onError: (error) => {
                toast.error('Failed to save Publish settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={updatePublishSettings.isPending}
          size="sm"
        >
          {updatePublishSettings.isPending ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Save className="mr-2 h-4 w-4" />
          )}
          Save Settings
        </Button>
      </div>
    </ConfigurationSectionWrapper>
  )
}
