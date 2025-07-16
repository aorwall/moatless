import type { CodeExecution, SetupScriptCommand, SetupScriptResponse } from '@moatless/types'
import { CheckCircle, Play, Terminal, XCircle } from 'lucide-react'
import { useId, useState } from 'react'
import { useRunSetupScript } from '@/features/repositories/hooks/use-repositories'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/lib/components/ui/sheet'
import { Textarea } from '@/lib/components/ui/textarea'

interface CodeExecutionConfigProps {
  codeExecution: CodeExecution
  onUpdate: (codeExecution: CodeExecution) => void
  isSubmitting?: boolean
  repositoryId?: string
}

interface SetupOutputSheetProps {
  showSetupOutput: boolean
  setShowSetupOutput: (show: boolean) => void
  setupScriptResult: SetupScriptResponse | null
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

function SetupOutputSheet({
  showSetupOutput,
  setShowSetupOutput,
  setupScriptResult,
}: SetupOutputSheetProps) {
  return (
    <Sheet open={showSetupOutput} onOpenChange={setShowSetupOutput}>
      <SheetContent className="w-full sm:max-w-2xl">
        <SheetHeader>
          <SheetTitle className="flex items-center gap-2">
            <Terminal className="h-5 w-5" />
            Setup Script Output
          </SheetTitle>
          <SheetDescription>
            {setupScriptResult && (
              <div className="flex items-center gap-2 mt-2">
                {setupScriptResult.success ? (
                  <>
                    <CheckCircle className="h-4 w-4 text-status-success" />
                    <span className="text-status-success">All commands succeeded</span>
                  </>
                ) : (
                  <>
                    <XCircle className="h-4 w-4 text-destructive" />
                    <span className="text-destructive">Some commands failed</span>
                  </>
                )}
              </div>
            )}
          </SheetDescription>
        </SheetHeader>
        <ScrollArea className="h-[calc(100vh-10rem)] mt-6">
          {setupScriptResult && (
            <div className="space-y-4">
              {setupScriptResult.commands.map((command: SetupScriptCommand, index: number) => (
                <div
                  key={`command-${index}-${command.command.slice(0, 20)}`}
                  className={`rounded-lg border ${
                    command.success ? 'border-status-success/20' : 'border-status-error/20'
                  } p-4 space-y-2`}
                >
                  <div className="flex items-start justify-between gap-2">
                    <code className="text-sm font-mono bg-muted px-2 py-1 rounded flex-1">
                      $ {command.command}
                    </code>
                    <div className="flex items-center gap-2">
                      {command.success ? (
                        <CheckCircle className="h-4 w-4 text-status-success flex-shrink-0" />
                      ) : (
                        <XCircle className="h-4 w-4 text-destructive flex-shrink-0" />
                      )}
                      <span className="text-xs text-muted-foreground">
                        Exit code: {command.exit_code}
                      </span>
                    </div>
                  </div>
                  {command.output && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium text-muted-foreground">Output:</div>
                      <pre className="text-xs bg-muted p-3 rounded overflow-auto whitespace-pre-wrap max-h-64">
                        {command.output}
                      </pre>
                    </div>
                  )}
                  {command.error && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium text-destructive">Error:</div>
                      <pre className="text-xs bg-destructive/10 text-destructive p-3 rounded overflow-auto whitespace-pre-wrap max-h-64">
                        {command.error}
                      </pre>
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </ScrollArea>
      </SheetContent>
    </Sheet>
  )
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
  repositoryId,
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
  const [showSetupOutput, setShowSetupOutput] = useState(false)
  const [setupScriptResult, setSetupScriptResult] = useState<SetupScriptResponse | null>(null)

  const runSetupScript = useRunSetupScript({
    onSuccess: (data: SetupScriptResponse) => {
      setSetupScriptResult(data)
      setShowSetupOutput(true)
    },
  })

  const buildCodeExecution = (): CodeExecution => {
    const updatedExecution: CodeExecution = {
      type: formData.type,
      setupCommands: formData.setup_commands
        ? formData.setup_commands
            .split('\n')
            .map((cmd) => cmd.trim())
            .filter((cmd) => cmd.length > 0)
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

  const handleValidate = () => {
    // First save the configuration
    onUpdate(buildCodeExecution())

    // Then run the setup script if repository ID is available
    if (repositoryId) {
      runSetupScript.mutate(repositoryId)
    }
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
              {repositoryId && (
                <Button
                  type="button"
                  variant="outline"
                  onClick={handleValidate}
                  disabled={isSubmitting || runSetupScript.isPending}
                >
                  <Play className="mr-2 h-4 w-4" />
                  {runSetupScript.isPending ? 'Running...' : 'Validate'}
                </Button>
              )}
            </div>
          </form>
        </CardContent>
      </Card>

      <SetupOutputSheet
        showSetupOutput={showSetupOutput}
        setShowSetupOutput={setShowSetupOutput}
        setupScriptResult={setupScriptResult}
      />
    </>
  )
}
