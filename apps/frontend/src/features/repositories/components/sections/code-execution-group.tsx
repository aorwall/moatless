import type { UpdateRepositoryRequest } from '@moatless/types'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSelect,
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
      <ConfigurationItem
        label="Execution Type"
        description="Choose how to run code in this repository"
      >
        <ConfigurationSelect
          value={formData.codeExecution?.type || 'local'}
          onValueChange={(value) =>
            updateField('codeExecution', {
              ...formData.codeExecution,
              type: value as 'local' | 'docker',
            })
          }
          options={[
            { value: 'local', label: 'Local' },
            { value: 'docker', label: 'Docker' },
          ]}
        />
      </ConfigurationItem>

      {formData.codeExecution?.type === 'docker' && (
        <ConfigurationItem
          label="Docker Image"
          description="Docker image to use for code execution"
        >
          <ConfigurationInput
            value={formData.codeExecution.dockerImage || ''}
            onChange={(e) =>
              updateField('codeExecution', {
                type: formData.codeExecution?.type || 'local',
                ...formData.codeExecution,
                dockerImage: e.target.value,
              })
            }
            placeholder="e.g., node:20-alpine"
          />
        </ConfigurationItem>
      )}

      <ConfigurationItem label="Setup Commands" description="Commands to run before executing code">
        <ConfigurationTextarea
          value={formData.codeExecution?.setupCommands?.join('\n') || ''}
          onChange={(e) =>
            updateField('codeExecution', {
              type: formData.codeExecution?.type || 'local',
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
              type: formData.codeExecution?.type || 'local',
              ...formData.codeExecution,
              envVars: vars,
            })
          }
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
