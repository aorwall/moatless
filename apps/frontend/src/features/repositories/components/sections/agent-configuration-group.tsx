import type { UpdateRepositoryRequest } from '@moatless/types'
import { Info } from 'lucide-react'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'

interface AgentConfigurationGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
  isConfigFileOnly?: boolean
}

export function AgentConfigurationGroup({
  formData,
  onUpdateField,
  isConfigFileOnly,
}: AgentConfigurationGroupProps) {
  const updateField = onUpdateField

  return (
    <ConfigurationGroup title="Agent Configuration" showSeparator>
      {isConfigFileOnly && (
        <Alert>
          <Info className="h-4 w-4" />
          <AlertDescription>
            Configuration is synced from repository file and clone mode is enabled. To update these
            settings, edit <code>.moatless/config.json</code> in your repository.
          </AlertDescription>
        </Alert>
      )}
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
          disabled={isConfigFileOnly}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
