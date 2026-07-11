import type { UpdateRepositoryRequest } from '@moatless/api-client'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { EnvironmentVariablesList } from '../environment-variables-list'

interface CodeExecutionGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function CodeExecutionGroup({ formData, onUpdateField }: CodeExecutionGroupProps) {
  const updateField = onUpdateField

  const envVars = formData.envVars ?? {}

  const setupCommands = formData.setupCommands ?? []

  return (
    <ConfigurationGroup title="Code Execution" showSeparator>
      <ConfigurationItem
        label="Docker Image"
        description="Custom Docker image for the workspace container"
      >
        <ConfigurationInput
          value={formData.dockerImage || ''}
          onChange={(e) => updateField('dockerImage', e.target.value || null)}
          placeholder="aorwall/moatless-vibe-sandbox-js:latest"
        />
      </ConfigurationItem>

      <ConfigurationItem label="Setup Commands" description="Commands to run before executing code">
        <ConfigurationTextarea
          value={setupCommands.join('\n')}
          onChange={(e) => updateField('setupCommands', e.target.value.split('\n'))}
          placeholder="npm install&#10;npm run build"
          rows={4}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Environment Variables"
        description="Variables available during code execution"
      >
        <EnvironmentVariablesList
          variables={envVars}
          onChange={(vars) => updateField('envVars', vars)}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
