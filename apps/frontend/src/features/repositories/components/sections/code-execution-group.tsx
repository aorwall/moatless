import type { UpdateRepositoryRequest } from '@moatless/types'
import { Info } from 'lucide-react'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { EnvironmentVariablesList } from '../environment-variables-list'

interface CodeExecutionGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
  isConfigFileOnly?: boolean
}

export function CodeExecutionGroup({
  formData,
  onUpdateField,
  isConfigFileOnly,
}: CodeExecutionGroupProps) {
  const updateField = onUpdateField

  return (
    <ConfigurationGroup title="Code Execution" showSeparator>
      {isConfigFileOnly && (
        <Alert>
          <Info className="h-4 w-4" />
          <AlertDescription>
            Configuration is synced from repository file and clone mode is enabled. To update these
            settings, edit <code>.moatless/config.json</code> in your repository.
          </AlertDescription>
        </Alert>
      )}

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
          disabled={isConfigFileOnly}
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
          disabled={isConfigFileOnly}
        />
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
