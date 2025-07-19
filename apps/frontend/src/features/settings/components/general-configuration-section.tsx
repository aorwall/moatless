import type { UpdateGeneralSettingsRequest } from '@moatless/types'
import { FolderOpen, Loader2, RotateCcw, Save } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useGeneralSettings, useUpdateGeneralSettings } from '@/features/settings'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper,
  ConfigurationSelect,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

export function GeneralConfigurationSection() {
  const {
    data: generalSettings,
    isLoading: generalLoading,
    error: generalError,
  } = useGeneralSettings()
  const updateGeneralSettings = useUpdateGeneralSettings()

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
    <ConfigurationSectionWrapper>
      <ConfigurationGroup>
        <ConfigurationItem
          label="Data Directory"
          description="Directory where application data is stored"
        >
          <div className="flex gap-2">
            <ConfigurationInput
              type="text"
              placeholder={generalSettings?.defaults.dataDir || '~/.moatless'}
              value={generalFormData.dataDir || ''}
              className="flex-1"
              onChange={(e) => {
                const value = e.target.value.trim()
                setGeneralUserModifications((prev) => ({
                  ...prev,
                  dataDir:
                    value === '' || value === generalSettings?.defaults.dataDir ? null : value,
                }))
              }}
            />
            <Button type="button" variant="outline" size="icon" title="Browse folders" disabled>
              <FolderOpen className="h-4 w-4" />
            </Button>
          </div>
        </ConfigurationItem>

        <ConfigurationItem
          label="Default Workspace Type"
          description="Default workspace type for new tasks"
        >
          <ConfigurationSelect
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
            options={[
              { value: 'local', label: 'Local' },
              { value: 'docker', label: 'Docker' },
            ]}
            placeholder="Select workspace type"
          />
        </ConfigurationItem>
      </ConfigurationGroup>

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
    </ConfigurationSectionWrapper>
  )
}
