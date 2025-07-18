import type { UpdateClaudeSettingsRequest } from '@moatless/types'
import { Eye, EyeOff, Loader2, RotateCcw, Save } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useClaudeSettings, useUpdateClaudeSettings } from '@/features/settings'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Separator } from '@/lib/components/ui/separator'
import { getErrorMessage } from '@/lib/utils/error'

export function ClaudeConfigurationSection() {
  const { data: claudeSettings, isLoading: claudeLoading, error: claudeError } = useClaudeSettings()
  const updateClaudeSettings = useUpdateClaudeSettings()

  const binDirId = useId()
  const configDirId = useId()
  const oauthTokenId = useId()
  const [showToken, setShowToken] = useState(false)

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseClaudeFormData = useMemo<UpdateClaudeSettingsRequest>(
    () => ({
      binDir: claudeSettings?.binDir || null,
      configDir: claudeSettings?.configDir || null,
      oauthToken: claudeSettings?.oauthToken || null,
    }),
    [claudeSettings],
  )

  // Track user modifications separately from the derived data
  const [claudeUserModifications, setClaudeUserModifications] = useState<
    Partial<UpdateClaudeSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const claudeFormData = useMemo(
    () => ({
      ...baseClaudeFormData,
      ...claudeUserModifications,
    }),
    [baseClaudeFormData, claudeUserModifications],
  )

  if (claudeLoading) {
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

  return (
    <div className="space-y-6">
      {/* Claude Binary Directory */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={binDirId} className="text-sm font-medium">
            Claude Bin Directory
          </Label>
          <p className="text-sm text-muted-foreground">Directory containing Claude Code binary</p>
        </div>
        <Input
          id={binDirId}
          type="text"
          placeholder="Auto-detect (optional)"
          value={claudeFormData.binDir || ''}
          onChange={(e) => {
            const value = e.target.value.trim()
            setClaudeUserModifications((prev) => ({
              ...prev,
              binDir: value === '' ? null : value,
            }))
          }}
        />
      </div>

      {/* Claude Config Directory */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={configDirId} className="text-sm font-medium">
            Claude Config Directory
          </Label>
          <p className="text-sm text-muted-foreground">Directory for Claude Code configuration</p>
        </div>
        <Input
          id={configDirId}
          type="text"
          placeholder="Auto-detect (optional)"
          value={claudeFormData.configDir || ''}
          onChange={(e) => {
            const value = e.target.value.trim()
            setClaudeUserModifications((prev) => ({
              ...prev,
              configDir: value === '' ? null : value,
            }))
          }}
        />
      </div>

      {/* OAuth Token */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={oauthTokenId} className="text-sm font-medium">
            Claude OAuth Token
          </Label>
          <p className="text-sm text-muted-foreground">
            OAuth token for Claude Code authentication (run `claude setup-token` to get it)
          </p>
        </div>
        <div className="flex gap-2">
          <Input
            id={oauthTokenId}
            type={showToken ? 'text' : 'password'}
            placeholder={
              claudeFormData.oauthToken ? 'Token configured' : 'Enter your Claude OAuth token'
            }
            value={claudeFormData.oauthToken || ''}
            className="flex-1 font-mono"
            onChange={(e) => {
              const value = e.target.value.trim()
              setClaudeUserModifications((prev) => ({
                ...prev,
                oauthToken: value === '' ? null : value,
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
      </div>

      <Separator />

      {/* Actions */}
      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between pt-4">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            // Reset Claude settings to defaults by clearing modifications
            setClaudeUserModifications({})
            toast.success('Claude settings reset to defaults')
          }}
          disabled={false}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset to Defaults
        </Button>
        <Button
          onClick={() => {
            updateClaudeSettings.mutate(claudeFormData, {
              onSuccess: () => {
                setClaudeUserModifications({})
                toast.success('Claude settings saved successfully')
              },
              onError: (error) => {
                toast.error('Failed to save Claude settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={updateClaudeSettings.isPending}
          size="sm"
        >
          {updateClaudeSettings.isPending ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Save className="mr-2 h-4 w-4" />
          )}
          Save Settings
        </Button>
      </div>
    </div>
  )
}
