import type { UpdateRepositoryRequest } from '@moatless/types'
import {
  ConfigurationGroup,
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

  return (
    <ConfigurationGroup title="Code Execution" showSeparator>
      <ConfigurationItem label="Setup Commands" description="Commands to run before executing code">
        <ConfigurationTextarea
          value={formData.codeExecution?.setupCommands?.join('\n') || ''}
          onChange={(e) =>
            updateField('codeExecution', {
              ...formData.codeExecution,
              setupCommands: e.target.value.split('\n'),
            })
          }
          placeholder="npm install&#10;npm run build"
          rows={4}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Environment Variables"
        description="Variables available during code execution"
      >
        <EnvironmentVariablesList
          variables={formData.codeExecution?.envVars || {}}
          onChange={(vars) =>
            updateField('codeExecution', {
              ...formData.codeExecution,
              envVars: vars,
            })
          }
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
