import type { CodeExecution } from '@moatless/types'
import { useId, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Textarea } from '@/lib/components/ui/textarea'

interface CodeExecutionConfigProps {
  codeExecution: CodeExecution
  onUpdate: (codeExecution: CodeExecution) => void
  isSubmitting?: boolean
}

interface EnvironmentVariablesProps {
  envVars: Record<string, string>
  envVarKey: string
  envVarValue: string
  setEnvVarKey: (key: string) => void
  setEnvVarValue: (value: string) => void
  onAddEnvVar: () => void
  onRemoveEnvVar: (key: string) => void
}

function EnvironmentVariables({
  envVars,
  envVarKey,
  envVarValue,
  setEnvVarKey,
  setEnvVarValue,
  onAddEnvVar,
  onRemoveEnvVar,
}: EnvironmentVariablesProps) {
  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <Label>Environment Variables</Label>
      </div>
      <div className="space-y-2">
        {Object.entries(envVars).map(([key, value]) => (
          <div key={key} className="flex items-center gap-2">
            <Input value={key} disabled className="flex-1" />
            <Input value={value} disabled className="flex-1" />
            <Button type="button" variant="outline" size="sm" onClick={() => onRemoveEnvVar(key)}>
              Remove
            </Button>
          </div>
        ))}
        <div className="flex items-center gap-2">
          <Input
            placeholder="Key"
            value={envVarKey}
            onChange={(e) => setEnvVarKey(e.target.value)}
            className="flex-1"
          />
          <Input
            placeholder="Value"
            value={envVarValue}
            onChange={(e) => setEnvVarValue(e.target.value)}
            className="flex-1"
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onAddEnvVar}
            disabled={!(envVarKey && envVarValue)}
          >
            Add
          </Button>
        </div>
      </div>
    </div>
  )
}

export function CodeExecutionConfig({
  codeExecution,
  onUpdate,
  isSubmitting,
}: CodeExecutionConfigProps) {
  const executionTypeId = useId()
  const imageId = useId()
  const setupCommandsId = useId()

  const [formData, setFormData] = useState({
    type: codeExecution.type,
    setup_commands: Array.isArray(codeExecution.setupCommands)
      ? codeExecution.setupCommands.join('\n')
      : '',
    env_vars: codeExecution.envVars || {},
    dockerImage: codeExecution.dockerImage || 'moatless-vibe-universal:latest',
  })

  const [envVarKey, setEnvVarKey] = useState('')
  const [envVarValue, setEnvVarValue] = useState('')

  const buildCodeExecution = (): CodeExecution => {
    const updatedExecution: CodeExecution = {
      type: formData.type,
      setupCommands: formData.setup_commands
        ? formData.setup_commands
            .split('\n')
            .map((cmd: string) => cmd.trim())
            .filter((cmd: string) => cmd.length > 0)
        : undefined,
      envVars: Object.keys(formData.env_vars).length > 0 ? formData.env_vars : undefined,
    }

    if (formData.type === 'docker') {
      updatedExecution.dockerImage = formData.dockerImage
    }

    return updatedExecution
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    onUpdate(buildCodeExecution())
  }

  const handleAddEnvVar = () => {
    if (envVarKey && envVarValue) {
      setFormData({
        ...formData,
        env_vars: {
          ...formData.env_vars,
          [envVarKey]: envVarValue,
        },
      })
      setEnvVarKey('')
      setEnvVarValue('')
    }
  }

  const handleRemoveEnvVar = (key: string) => {
    const { [key]: _, ...rest } = formData.env_vars
    setFormData({
      ...formData,
      env_vars: rest,
    })
  }

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Code Execution Settings</CardTitle>
          <CardDescription>Configure how code will be executed in this repository</CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-4">
              <div>
                <Label htmlFor={executionTypeId}>Execution Type</Label>
                <Select
                  value={formData.type}
                  onValueChange={(value: 'local' | 'docker') =>
                    setFormData({ ...formData, type: value })
                  }
                >
                  <SelectTrigger id={executionTypeId}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="local">Local</SelectItem>
                    <SelectItem value="docker">Docker</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-sm text-muted-foreground mt-1">
                  Choose between local execution or Docker containerized execution
                </p>
              </div>

              {formData.type === 'docker' && (
                <div>
                  <Label htmlFor={imageId}>Docker Image</Label>
                  <Input
                    id={imageId}
                    value={formData.dockerImage}
                    onChange={(e) => setFormData({ ...formData, dockerImage: e.target.value })}
                    placeholder="moatless-vibe-universal:latest"
                    required
                  />
                </div>
              )}

              <div>
                <Label htmlFor={setupCommandsId}>Setup Commands</Label>
                <Textarea
                  id={setupCommandsId}
                  value={formData.setup_commands}
                  onChange={(e) => setFormData({ ...formData, setup_commands: e.target.value })}
                  placeholder="pip install -r requirements.txt&#10;npm install"
                  rows={4}
                />
                <p className="text-sm text-muted-foreground mt-1">
                  Commands to run when setting up the repository (one per line)
                </p>
              </div>

              <EnvironmentVariables
                envVars={formData.env_vars}
                envVarKey={envVarKey}
                envVarValue={envVarValue}
                setEnvVarKey={setEnvVarKey}
                setEnvVarValue={setEnvVarValue}
                onAddEnvVar={handleAddEnvVar}
                onRemoveEnvVar={handleRemoveEnvVar}
              />
            </div>

            <div className="flex justify-end gap-2">
              <Button type="submit" disabled={isSubmitting}>
                {isSubmitting ? 'Saving...' : 'Save Changes'}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </>
  )
}
