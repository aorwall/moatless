import type { UpdateGeneralSettingsRequest } from '@moatless/types'
import { FolderOpen, Loader2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useGeneralSettings } from '@/features/settings'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper,
  ConfigurationSelect,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'

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
    </ConfigurationSectionWrapper>
  )
}
