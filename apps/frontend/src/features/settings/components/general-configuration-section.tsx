import type { UpdateGeneralSettingsRequest } from '@moatless/types'
import { FolderOpen, Loader2, RotateCcw, Save } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useGeneralSettings, useUpdateGeneralSettings } from '@/features/settings'
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
    data: generalSettings,
    isLoading: generalLoading,
    error: generalError,
  } = useGeneralSettings()
  const updateGeneralSettings = useUpdateGeneralSettings()

  const dataDirId = useId()
  const defaultWorkspaceTypeId = useId()

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseGeneralFormData = useMemo<UpdateGeneralSettingsRequest>(
    () => ({
      dataDir: generalSettings?.settings.dataDir || null,
      defaultWorkspaceType: generalSettings?.settings.defaultWorkspaceType || null,
    }),
    [generalSettings],
  )

  // Track user modifications separately from the derived data
  const [generalUserModifications, setGeneralUserModifications] = useState<
    Partial<UpdateGeneralSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const generalFormData = useMemo(
    () => ({
      ...baseGeneralFormData,
      ...generalUserModifications,
    }),
    [baseGeneralFormData, generalUserModifications],
  )

  if (generalLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (generalError) {
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
            placeholder={generalSettings?.defaults.dataDir || '~/.moatless'}
            value={generalFormData.dataDir || ''}
            className="flex-1"
            onChange={(e) => {
              const value = e.target.value.trim()
              setGeneralUserModifications((prev) => ({
                ...prev,
                dataDir: value === '' || value === generalSettings?.defaults.dataDir ? null : value,
              }))
            }}
          />
          <Button type="button" variant="outline" size="icon" title="Browse folders" disabled>
            <FolderOpen className="h-4 w-4" />
          </Button>
        </div>
      </div>

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
            generalFormData.defaultWorkspaceType ||
            generalSettings?.defaults.defaultWorkspaceType ||
            'local'
          }
          onValueChange={(value) =>
            setGeneralUserModifications((prev) => ({
              ...prev,
              defaultWorkspaceType:
                value === generalSettings?.defaults.defaultWorkspaceType
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

      {/* Actions */}
      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between pt-4">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            // Reset to defaults by clearing modifications
            setGeneralUserModifications({})
            toast.success('Settings reset to defaults')
          }}
          disabled={false}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset to Defaults
        </Button>
        <Button
          onClick={() => {
            updateGeneralSettings.mutate(generalFormData, {
              onSuccess: () => {
                setGeneralUserModifications({})
                toast.success('Settings saved successfully')
              },
              onError: (error) => {
                toast.error('Failed to save settings', {
                  description: getErrorMessage(error),
                })
              },
            })
          }}
          disabled={updateGeneralSettings.isPending}
          size="sm"
        >
          {updateGeneralSettings.isPending ? (
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
