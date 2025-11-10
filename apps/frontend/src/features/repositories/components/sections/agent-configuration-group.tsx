import type { UpdateRepositoryRequest } from '@moatless/types'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationTextarea,
} from '@/lib/components/configuration'

interface AgentConfigurationGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function AgentConfigurationGroup({ formData, onUpdateField }: AgentConfigurationGroupProps) {
  const updateField = onUpdateField

  return (
    <ConfigurationGroup title="Agent Configuration" showSeparator>
      <ConfigurationItem
        label="System Prompt"
        description="Additional instructions to append to Claude Code's default system prompt. Use this to customize agent behavior for this repository (e.g., coding standards, preferred libraries, project-specific conventions)."
        className="grid-cols-1"
      >
        <ConfigurationTextarea
          value={formData.systemPrompt || ''}
          onChange={(e) => updateField('systemPrompt', e.target.value || undefined)}
          placeholder="e.g., Always use TypeScript strict mode. Prefer functional components."
          rows={6}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
