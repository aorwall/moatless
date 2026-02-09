import type { UpdateRepositoryRequest } from '@moatless/api-client'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSelect,
  ConfigurationTextarea,
} from '@/lib/components/configuration'

interface GeneralSettingsGroupProps {
  repositoryKind: string
  provider?: string | null
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function GeneralSettingsGroup({
  repositoryKind,
  provider,
  formData,
  onUpdateField,
}: GeneralSettingsGroupProps) {
  return (
    <ConfigurationGroup title="General Settings">
      <ConfigurationItem label="Repository Name" description="Display name for your repository">
        <ConfigurationInput
          value={formData.name || ''}
          onChange={(e) => onUpdateField('name', e.target.value)}
        />
      </ConfigurationItem>

      <ConfigurationItem label="Description" description="Optional description of this repository">
        <ConfigurationTextarea
          value={formData.description || ''}
          onChange={(e) => onUpdateField('description', e.target.value)}
          rows={3}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Visibility"
        description="Control who can access this repository. Private repositories are only visible to you, while public repositories can be viewed by all authenticated users."
      >
        <ConfigurationSelect
          value={formData.visibility || 'private'}
          onValueChange={(value) => onUpdateField('visibility', value)}
          options={[
            {
              value: 'private',
              label: 'Private',
            },
            {
              value: 'public',
              label: 'Public',
            },
          ]}
        />
      </ConfigurationItem>

      {/* Remote and Template repositories have URL and provider */}
      <ConfigurationItem
        label="Remote URL"
        description={
          repositoryKind === 'template'
            ? 'Template source repository URL (changing this will point to a different template)'
            : 'GitHub repository URL (changing this will point to a different repository)'
        }
      >
        <ConfigurationInput
          value={formData.remoteUrl || ''}
          onChange={(e) => onUpdateField('remoteUrl', e.target.value || null)}
          placeholder="https://github.com/owner/repo"
        />
      </ConfigurationItem>

      <ConfigurationItem label="Provider" description="Git provider type (GitHub or GitLab)">
        <ConfigurationInput value={provider || ''} disabled className="bg-muted" />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
