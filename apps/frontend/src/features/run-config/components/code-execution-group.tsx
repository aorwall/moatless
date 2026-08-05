import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import type { RunConfigFormValues, UpdateRunConfigField } from '../types'
import { EnvironmentVariablesList } from './environment-variables-list'

interface CodeExecutionGroupProps {
  formData: RunConfigFormValues
  onUpdateField: UpdateRunConfigField
  disabled?: boolean
}

export function CodeExecutionGroup({
  formData,
  onUpdateField,
  disabled = false,
}: CodeExecutionGroupProps) {
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
          placeholder="aorwall/moatless-sandbox:latest"
          disabled={disabled}
        />
      </ConfigurationItem>

      <ConfigurationItem label="Setup Commands" description="Commands to run before executing code">
        <ConfigurationTextarea
          value={setupCommands.join('\n')}
          onChange={(e) => updateField('setupCommands', e.target.value.split('\n'))}
          placeholder="npm install&#10;npm run build"
          rows={4}
          disabled={disabled}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Environment Variables"
        description="Variables available during code execution"
      >
        <EnvironmentVariablesList
          variables={envVars}
          onChange={(vars) => updateField('envVars', vars)}
          disabled={disabled}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
