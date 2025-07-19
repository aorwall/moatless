import type { UpdateGitHubSettingsRequest } from '@moatless/types'
import { Eye, EyeOff, Loader2, RotateCcw, Save } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useGitHubSettings, useUpdateGitHubSettings } from '@/features/settings'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

export function GitHubConfigurationSection() {
  const { data: githubSettings, isLoading: githubLoading, error: githubError } = useGitHubSettings()
  const updateGitHubSettings = useUpdateGitHubSettings()

  const [showToken, setShowToken] = useState(false)

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseGitHubFormData = useMemo<UpdateGitHubSettingsRequest>(
    () => ({
      personalAccessToken: githubSettings?.settings.personalAccessToken || null,
      userName: githubSettings?.settings.userName || null,
      userEmail: githubSettings?.settings.userEmail || null,
    }),
    [githubSettings],
  )

  // Track user modifications separately from the derived data
  const [githubUserModifications, setGitHubUserModifications] = useState<
    Partial<UpdateGitHubSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const githubFormData = useMemo(
    () => ({
      ...baseGitHubFormData,
      ...githubUserModifications,
    }),
    [baseGitHubFormData, githubUserModifications],
  )

  if (githubLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (githubError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load GitHub settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Configure GitHub authentication for accessing private repositories
        </p>

        <ConfigurationItem
          label="Personal Access Token"
          description="GitHub PAT for cloning private repositories and pushing changes"
        >
          <div className="flex gap-2">
            <ConfigurationInput
              type={showToken ? 'text' : 'password'}
              placeholder={
                githubFormData.personalAccessToken
                  ? 'Token configured'
                  : 'Enter your GitHub personal access token'
              }
              value={githubFormData.personalAccessToken || ''}
              className="flex-1 font-mono"
              onChange={(e) => {
                const value = e.target.value.trim()
                setGitHubUserModifications((prev) => ({
                  ...prev,
                  personalAccessToken: value === '' ? null : value,
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

        <ConfigurationItem
          label="Git User Name"
          description="Your name for Git commits (fetched from GitHub when PAT is saved)"
        >
          <ConfigurationInput
            type="text"
            placeholder={githubSettings?.defaults.userName || 'Enter your name'}
            value={githubFormData.userName || ''}
            className="flex-1"
            onChange={(e) => {
              const value = e.target.value.trim()
              setGitHubUserModifications((prev) => ({
                ...prev,
                userName:
                  value === '' || value === githubSettings?.defaults.userName ? null : value,
              }))
            }}
          />
        </ConfigurationItem>

        <ConfigurationItem
          label="Git User Email"
          description="Your email for Git commits (fetched from GitHub when PAT is saved)"
        >
          <ConfigurationInput
            type="email"
            placeholder={githubSettings?.defaults.userEmail || 'Enter your email'}
            value={githubFormData.userEmail || ''}
            className="flex-1"
            onChange={(e) => {
              const value = e.target.value.trim()
              setGitHubUserModifications((prev) => ({
                ...prev,
                userEmail:
                  value === '' || value === githubSettings?.defaults.userEmail ? null : value,
              }))
            }}
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Info Alert */}
      <Alert>
        <AlertTitle>How to create a Personal Access Token</AlertTitle>
        <AlertDescription className="space-y-2">
          <ol className="list-decimal list-inside space-y-1 text-sm">
            <li>
              Go to{' '}
              <a
                href="https://github.com/settings/tokens/new"
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-600 hover:underline"
              >
                GitHub Settings → Developer settings → Personal access tokens
              </a>
            </li>
            <li>Click "Generate new token (classic)"</li>
            <li>Give your token a descriptive name</li>
            <li>Select the "repo" scope for full control of private repositories</li>
            <li>Click "Generate token" and copy the token</li>
          </ol>
          <p className="text-sm text-muted-foreground mt-2">
            The token will be used to authenticate Git operations in Docker containers without SSH
            keys. Your GitHub username and email will be automatically fetched when you save the
            token.
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
            // Reset GitHub settings to defaults by clearing modifications
            setGitHubUserModifications({})
            toast.success('GitHub settings reset to defaults')
          }}
          disabled={false}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset to Defaults
        </Button>
        <Button
          onClick={() => {
            updateGitHubSettings.mutate(githubFormData, {
              onSuccess: () => {
                setGitHubUserModifications({})
                toast.success('GitHub settings saved successfully')
              },
              onError: (error) => {
                toast.error('Failed to save GitHub settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={updateGitHubSettings.isPending}
          size="sm"
        >
          {updateGitHubSettings.isPending ? (
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
