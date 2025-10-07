import type { UpdateRepositoryRequest } from '@moatless/types'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'

interface GeneralSettingsGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function GeneralSettingsGroup({ formData, onUpdateField }: GeneralSettingsGroupProps) {
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
        label="Git Remote URL"
        description="Remote repository URL (optional, enables cloning feature)"
      >
        <ConfigurationInput
          value={formData.remoteUrl || ''}
          onChange={(e) => onUpdateField('remoteUrl', e.target.value || undefined)}
          placeholder="https://github.com/owner/repo or git@github.com:owner/repo.git"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Repository Path"
        description="Local path to your repository (disabled when cloning on each task)"
      >
        <ConfigurationInput
          value={formData.path || ''}
          onChange={(e) => onUpdateField('path', e.target.value)}
          disabled={formData.cloneOnNewTask}
          className={formData.cloneOnNewTask ? 'bg-muted' : ''}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Git Worktrees"
        description="Use git worktrees for isolated environments"
      >
        <ConfigurationSwitch
          checked={formData.useGitWorktrees}
          onCheckedChange={(checked) => {
            onUpdateField('useGitWorktrees', checked)
            // Disable cloneOnNewTask when worktrees are enabled
            if (checked) {
              onUpdateField('cloneOnNewTask', false)
            }
          }}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Clone on New Task"
        description="Requires a Git remote URL. Clone the entire repository for each new task instead of using git worktrees. Provides complete isolation between tasks."
      >
        <ConfigurationSwitch
          checked={formData.cloneOnNewTask}
          disabled={!formData.remoteUrl}
          onCheckedChange={(checked) => {
            onUpdateField('cloneOnNewTask', checked)
            // Disable worktrees when cloning is enabled
            if (checked) {
              onUpdateField('useGitWorktrees', false)
            }
          }}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
