import type {
  CodeExecution,
  SetupScriptCommand,
  SetupScriptResponse,
} from '@moatless/shared/types/repository'
import { AlertCircle, CheckCircle, HelpCircle, Play, Terminal, XCircle } from 'lucide-react'
import { useId, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/lib/components/ui/dialog'
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

import { useRunSetupScript } from '../hooks/use-repositories'

interface CodeExecutionConfigProps {
  codeExecution: CodeExecution
  onUpdate: (codeExecution: CodeExecution) => void
  isSubmitting?: boolean
  repositoryId?: string
}

interface ServerCommandExamplesProps {
  showServerExamples: boolean
  setShowServerExamples: (show: boolean) => void
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

function ServerCommandExamples({
  showServerExamples,
  setShowServerExamples,
}: ServerCommandExamplesProps) {
  const examples = [
    {
      title: 'Vite',
      command: 'npm run dev -- --port {PORT}',
      description: 'For Vite-based projects (Vue, React, etc.)',
    },
    {
      title: 'Next.js',
      command: 'next dev -p {PORT}',
      description: 'For Next.js applications',
    },
    {
      title: 'Python HTTP Server',
      command: 'python -m http.server {PORT}',
      description: 'Simple HTTP server for static files',
    },
    {
      title: 'Flask',
      command: 'flask run --port {PORT}',
      description: 'For Flask applications',
    },
    {
      title: 'Django',
      command: 'python manage.py runserver 0.0.0.0:{PORT}',
      description: 'For Django applications',
    },
    {
      title: 'Node.js (with nodemon)',
      command: 'nodemon --port {PORT} server.js',
      description: 'For Node.js applications with auto-reload',
    },
  ]

  return (
    <Dialog open={showServerExamples} onOpenChange={setShowServerExamples}>
      <DialogTrigger asChild>
        <Button type="button" variant="ghost" size="sm" className="h-5 w-5 p-0">
          <HelpCircle className="h-4 w-4" />
          <span className="sr-only">Show examples</span>
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Server Command Examples</DialogTitle>
          <DialogDescription>
            The server command should include{' '}
            <code className="bg-muted px-1 py-0.5 rounded">{'{PORT}'}</code> as a placeholder for
            the port number.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 mt-4">
          <div className="space-y-3">
            {examples.map((example) => (
              <div key={example.title} className="rounded-lg border p-4 space-y-2">
                <p className="font-medium">{example.title}</p>
                <code className="block bg-muted p-2 rounded text-sm">{example.command}</code>
                <p className="text-sm text-muted-foreground">{example.description}</p>
              </div>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
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
                    command.success
                      ? 'border-status-success/20'
                      : 'border-status-error/20'
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
  const serverCommandId = useId()

  const [formData, setFormData] = useState({
    type: codeExecution.type,
    setup_commands: codeExecution.setupCommands || '',
    server_command: codeExecution.serverCommand || '',
    env_vars: codeExecution.envVars || {},
    image: codeExecution.image || 'ubuntu:latest',
  })

  const [envVarKey, setEnvVarKey] = useState('')
  const [envVarValue, setEnvVarValue] = useState('')
  const [showSetupOutput, setShowSetupOutput] = useState(false)
  const [setupScriptResult, setSetupScriptResult] = useState<SetupScriptResponse | null>(null)
  const [serverCommandError, setServerCommandError] = useState<string | null>(null)
  const [showServerExamples, setShowServerExamples] = useState(false)

  const runSetupScript = useRunSetupScript({
    onSuccess: (data) => {
      setSetupScriptResult(data)
      setShowSetupOutput(true)
    },
  })

  const buildCodeExecution = (): CodeExecution => {
    const updatedExecution: CodeExecution = {
      type: formData.type,
      setupCommands: formData.setup_commands || undefined,
      serverCommand: formData.server_command || undefined,
      envVars: Object.keys(formData.env_vars).length > 0 ? formData.env_vars : undefined,
    }

    if (formData.type === 'docker') {
      updatedExecution.image = formData.image
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

  const handleServerCommandChange = (value: string) => {
    setFormData({ ...formData, server_command: value })
    // Validate server command
    if (value && !value.includes('{PORT}')) {
      setServerCommandError('Server command must include {PORT} placeholder')
    } else {
      setServerCommandError(null)
    }
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
                  disabled
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
                  Execution type cannot be changed after creation
                </p>
              </div>

              {formData.type === 'docker' && (
                <div>
                  <Label htmlFor={imageId}>Docker Image</Label>
                  <Input
                    id={imageId}
                    value={formData.image}
                    onChange={(e) => setFormData({ ...formData, image: e.target.value })}
                    placeholder="ubuntu:latest"
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

              <div>
                <div className="flex items-center gap-2 mb-2">
                  <Label htmlFor={serverCommandId}>Server Command</Label>
                  <ServerCommandExamples
                    showServerExamples={showServerExamples}
                    setShowServerExamples={setShowServerExamples}
                  />
                </div>
                <Input
                  id={serverCommandId}
                  value={formData.server_command}
                  onChange={(e) => handleServerCommandChange(e.target.value)}
                  placeholder="npm run dev -- --port {PORT}"
                  className={serverCommandError ? 'border-destructive' : ''}
                />
                <div className="space-y-1 mt-2">
                  {serverCommandError && (
                    <div className="flex items-center gap-2 text-sm text-destructive">
                      <AlertCircle className="h-4 w-4" />
                      <span>{serverCommandError}</span>
                    </div>
                  )}
                  <p className="text-sm text-muted-foreground">
                    Command to start a development server. Use{' '}
                    <code className="bg-muted px-1 py-0.5 rounded">{'{PORT}'}</code> as a
                    placeholder for the port number.
                  </p>
                </div>
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
              <Button type="submit" disabled={isSubmitting || !!serverCommandError}>
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
