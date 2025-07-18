import type { UpdateSettingsRequest } from '@moatless/types'
import { FolderOpen, Loader2, RotateCcw, Save } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useResetSettings, useSettings, useUpdateSettings } from '@/features/settings'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Separator } from '@/lib/components/ui/separator'
import { getErrorMessage } from '@/lib/utils/error'

export function GeneralConfigurationSection() {
  const {
    data: appSettings,
    isLoading: appSettingsLoading,
    error: appSettingsError,
  } = useSettings()
  const updateAppSettings = useUpdateSettings()
  const resetAppSettings = useResetSettings()

  const dataDirId = useId()
  const defaultWorkspaceTypeId = useId()
  const claudeBinDirId = useId()
  const claudeConfigDirId = useId()

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseAppFormData = useMemo<UpdateSettingsRequest>(
    () => ({
      dataDir: appSettings?.settings.dataDir || null,
      defaultWorkspaceType: appSettings?.settings.defaultWorkspaceType || null,
      claude: {
        binDir: appSettings?.settings.claude?.binDir || null,
        configDir: appSettings?.settings.claude?.configDir || null,
      },
    }),
    [appSettings],
  )

  // Track user modifications separately from the derived data
  const [appUserModifications, setAppUserModifications] = useState<Partial<UpdateSettingsRequest>>(
    {},
  )

  // Merge derived data with user modifications
  const appFormData = useMemo(
    () => ({
      ...baseAppFormData,
      ...appUserModifications,
      claude: {
        ...baseAppFormData.claude,
        ...appUserModifications.claude,
      },
    }),
    [baseAppFormData, appUserModifications],
  )

  if (appSettingsLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (appSettingsError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>
          Failed to load application settings. Please try again later.
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <div className="space-y-6">
      {/* Data Directory */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={dataDirId} className="text-sm font-medium">
            Data Directory
          </Label>
          <p className="text-sm text-muted-foreground">
            Directory where application data is stored
          </p>
        </div>
        <div className="flex gap-2">
          <Input
            id={dataDirId}
            type="text"
            placeholder={appSettings?.defaults.dataDir || '~/.moatless'}
            value={appFormData.dataDir || ''}
            className="flex-1"
            onChange={(e) => {
              const value = e.target.value.trim()
              setAppUserModifications((prev) => ({
                ...prev,
                dataDir: value === '' || value === appSettings?.defaults.dataDir ? null : value,
              }))
            }}
          />
          <Button type="button" variant="outline" size="icon" title="Browse folders" disabled>
            <FolderOpen className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <Separator />

      {/* Default Workspace Type */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
        <div className="space-y-1">
          <Label htmlFor={defaultWorkspaceTypeId} className="text-sm font-medium">
            Default Workspace Type
          </Label>
          <p className="text-sm text-muted-foreground">Default workspace type for new tasks</p>
        </div>
        <Select
          value={
            appFormData.defaultWorkspaceType ||
            appSettings?.defaults.defaultWorkspaceType ||
            'local'
          }
          onValueChange={(value) =>
            setAppUserModifications((prev) => ({
              ...prev,
              defaultWorkspaceType:
                value === appSettings?.defaults.defaultWorkspaceType
                  ? null
                  : (value as 'docker' | 'local'),
            }))
          }
        >
          <SelectTrigger id={defaultWorkspaceTypeId}>
            <SelectValue placeholder="Select workspace type" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="local">Local</SelectItem>
            <SelectItem value="docker">Docker</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <Separator />

      {/* Claude Directories */}
      <div className="space-y-6">
        <div className="space-y-1">
          <h3 className="text-base font-medium">Claude Code Configuration</h3>
          <p className="text-sm text-muted-foreground">
            Configure Claude Code binary and configuration directories
          </p>
        </div>

        <div className="space-y-6">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={claudeBinDirId} className="text-sm font-medium">
                Claude Bin Directory
              </Label>
              <p className="text-sm text-muted-foreground">
                Directory containing Claude Code binary
              </p>
            </div>
            <Input
              id={claudeBinDirId}
              type="text"
              placeholder={appSettings?.defaults.claude.binDir || 'Auto-detect (optional)'}
              value={appFormData.claude?.binDir || ''}
              onChange={(e) => {
                const value = e.target.value.trim()
                setAppUserModifications((prev) => ({
                  ...prev,
                  claude: {
                    ...prev.claude,
                    binDir:
                      value === '' || value === appSettings?.defaults.claude.binDir ? null : value,
                  },
                }))
              }}
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={claudeConfigDirId} className="text-sm font-medium">
                Claude Config Directory
              </Label>
              <p className="text-sm text-muted-foreground">
                Directory for Claude Code configuration
              </p>
            </div>
            <Input
              id={claudeConfigDirId}
              type="text"
              placeholder={appSettings?.defaults.claude.configDir || 'Auto-detect (optional)'}
              value={appFormData.claude?.configDir || ''}
              onChange={(e) => {
                const value = e.target.value.trim()
                setAppUserModifications((prev) => ({
                  ...prev,
                  claude: {
                    ...prev.claude,
                    configDir:
                      value === '' || value === appSettings?.defaults.claude.configDir
                        ? null
                        : value,
                  },
                }))
              }}
            />
          </div>
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
            resetAppSettings.mutate(undefined, {
              onSuccess: () => {
                setAppUserModifications({})
                toast.success('Application settings reset to defaults')
              },
              onError: (error) => {
                toast.error('Failed to reset application settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={resetAppSettings.isPending}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset to Defaults
        </Button>
        <Button
          onClick={() => {
            updateAppSettings.mutate(appFormData, {
              onSuccess: () => {
                setAppUserModifications({})
                toast.success('Application settings saved successfully')
              },
              onError: (error) => {
                toast.error('Failed to save application settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={updateAppSettings.isPending}
          size="sm"
        >
          {updateAppSettings.isPending ? (
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
