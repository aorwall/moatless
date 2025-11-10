import type { UpdateRepositoryRequest } from '@moatless/types'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'

interface GeneralSettingsGroupProps {
  repositoryKind: 'local' | 'remote' | 'template'
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function GeneralSettingsGroup({
  repositoryKind,
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

      {/* Local and Template repositories have path and worktrees */}
      {(repositoryKind === 'local' || repositoryKind === 'template') && (
        <>
          <ConfigurationItem
            label="Repository Path"
            description={
              repositoryKind === 'template'
                ? 'Path where template was cloned (read-only)'
                : 'Local path to your repository'
            }
          >
            <ConfigurationInput
              value={formData.path || ''}
              onChange={(e) => onUpdateField('path', e.target.value)}
              disabled={repositoryKind === 'template'}
              className={repositoryKind === 'template' ? 'bg-muted' : ''}
            />
          </ConfigurationItem>

          <ConfigurationItem
            label="Git Worktrees"
            description="Use git worktrees for isolated environments"
          >
            <ConfigurationSwitch
              checked={formData.useGitWorktrees ?? false}
              onCheckedChange={(checked) => onUpdateField('useGitWorktrees', checked)}
            />
          </ConfigurationItem>
        </>
      )}

      {/* Remote repositories have URL and provider */}
      {repositoryKind === 'remote' && (
        <>
          <ConfigurationItem
            label="Remote URL"
            description="GitHub repository URL (changing this will point to a different repository)"
          >
            <ConfigurationInput
              value={formData.remoteUrl || ''}
              onChange={(e) => onUpdateField('remoteUrl', e.target.value || undefined)}
              placeholder="https://github.com/owner/repo"
            />
          </ConfigurationItem>

          <ConfigurationItem
            label="Provider"
            description="Git provider type (currently only GitHub is supported)"
          >
            <ConfigurationInput
              value={formData.provider || 'github'}
              disabled
              className="bg-muted"
            />
          </ConfigurationItem>
        </>
      )}
    </ConfigurationGroup>
  )
}
