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

  // envVars from the API is typed as { [key: string]: unknown } | null,
  // but we treat it as Record<string, string> for the UI
  const envVars = (formData.envVars ?? {}) as Record<string, string>

  // setupCommands from the API is typed as { [key: string]: unknown } | null,
  // but we treat it as string[] for the UI
  const setupCommands = (formData.setupCommands ?? []) as unknown as string[]

  return (
    <ConfigurationGroup title="Code Execution" showSeparator>
      <ConfigurationItem
        label="Docker Image"
        description="Custom Docker image for the workspace container"
      >
        <ConfigurationInput
          value={formData.dockerImage || ''}
          onChange={(e) => updateField('dockerImage', e.target.value || null)}
          placeholder="ghcr.io/org/image:latest"
        />
      </ConfigurationItem>

      <ConfigurationItem label="Setup Commands" description="Commands to run before executing code">
        <ConfigurationTextarea
          value={setupCommands.join('\n')}
          onChange={(e) =>
            updateField(
              'setupCommands',
              e.target.value.split('\n') as unknown as UpdateRepositoryRequest['setupCommands'],
            )
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
          variables={envVars}
          onChange={(vars) =>
            updateField('envVars', vars as unknown as UpdateRepositoryRequest['envVars'])
          }
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
