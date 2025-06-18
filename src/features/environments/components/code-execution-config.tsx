import { CheckCircle, Play, Terminal, XCircle } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';
import { Input } from '@/lib/components/ui/input';
import { Label } from '@/lib/components/ui/label';
import { ScrollArea } from '@/lib/components/ui/scroll-area';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select';
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/lib/components/ui/sheet';
import { Textarea } from '@/lib/components/ui/textarea';
import type {
  CodeExecution,
  SetupScriptCommand,
  SetupScriptResponse,
} from '@/types/environment';

import { useRunSetupScript } from '../hooks/use-environments';

interface CodeExecutionConfigProps {
  codeExecution: CodeExecution;
  onUpdate: (codeExecution: CodeExecution) => void;
  isSubmitting?: boolean;
  environmentId?: string;
}

export function CodeExecutionConfig({
  codeExecution,
  onUpdate,
  isSubmitting,
  environmentId,
}: CodeExecutionConfigProps) {
  const [formData, setFormData] = useState({
    type: codeExecution.type,
    setup_commands: codeExecution.setup_commands || '',
    env_vars: codeExecution.env_vars || {},
    image: codeExecution.image || 'ubuntu:latest',
  });

  const [envVarKey, setEnvVarKey] = useState('');
  const [envVarValue, setEnvVarValue] = useState('');
  const [showSetupOutput, setShowSetupOutput] = useState(false);
  const [setupScriptResult, setSetupScriptResult] =
    useState<SetupScriptResponse | null>(null);

  const runSetupScript = useRunSetupScript({
    onSuccess: (data) => {
      setSetupScriptResult(data);
      setShowSetupOutput(true);
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    const updatedExecution: CodeExecution = {
      type: formData.type,
      setup_commands: formData.setup_commands || undefined,
      env_vars:
        Object.keys(formData.env_vars).length > 0
          ? formData.env_vars
          : undefined,
    };

    if (formData.type === 'docker') {
      updatedExecution.image = formData.image;
    }

    onUpdate(updatedExecution);
  };

  const handleValidate = () => {
    // First save the configuration
    const updatedExecution: CodeExecution = {
      type: formData.type,
      setup_commands: formData.setup_commands || undefined,
      env_vars:
        Object.keys(formData.env_vars).length > 0
          ? formData.env_vars
          : undefined,
    };

    if (formData.type === 'docker') {
      updatedExecution.image = formData.image;
    }

    onUpdate(updatedExecution);

    // Then run the setup script if environment ID is available
    if (environmentId) {
      runSetupScript.mutate(environmentId);
    }
  };

  const handleAddEnvVar = () => {
    if (envVarKey && envVarValue) {
      setFormData({
        ...formData,
        env_vars: {
          ...formData.env_vars,
          [envVarKey]: envVarValue,
        },
      });
      setEnvVarKey('');
      setEnvVarValue('');
    }
  };

  const handleRemoveEnvVar = (key: string) => {
    const { [key]: _, ...rest } = formData.env_vars;
    setFormData({
      ...formData,
      env_vars: rest,
    });
  };

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Code Execution Settings</CardTitle>
          <CardDescription>
            Configure how code will be executed in this environment
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-4">
              <div>
                <Label htmlFor="execution_type">Execution Type</Label>
                <Select
                  value={formData.type}
                  onValueChange={(value: 'local' | 'docker') =>
                    setFormData({ ...formData, type: value })
                  }
                  disabled
                >
                  <SelectTrigger id="execution_type">
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
                  <Label htmlFor="image">Docker Image</Label>
                  <Input
                    id="image"
                    value={formData.image}
                    onChange={(e) =>
                      setFormData({ ...formData, image: e.target.value })
                    }
                    placeholder="ubuntu:latest"
                    required
                  />
                </div>
              )}

              <div>
                <Label htmlFor="setup_commands">Setup Commands</Label>
                <Textarea
                  id="setup_commands"
                  value={formData.setup_commands}
                  onChange={(e) =>
                    setFormData({ ...formData, setup_commands: e.target.value })
                  }
                  placeholder="pip install -r requirements.txt&#10;npm install"
                  rows={4}
                />
                <p className="text-sm text-muted-foreground mt-1">
                  Commands to run when setting up the environment (one per line)
                </p>
              </div>

              <div>
                <div className="flex items-center justify-between mb-2">
                  <Label>Environment Variables</Label>
                </div>
                <div className="space-y-2">
                  {Object.entries(formData.env_vars).map(([key, value]) => (
                    <div key={key} className="flex items-center gap-2">
                      <Input value={key} disabled className="flex-1" />
                      <Input value={value} disabled className="flex-1" />
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => handleRemoveEnvVar(key)}
                      >
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
                      onClick={handleAddEnvVar}
                      disabled={!(envVarKey && envVarValue)}
                    >
                      Add
                    </Button>
                  </div>
                </div>
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <Button type="submit" disabled={isSubmitting}>
                {isSubmitting ? 'Saving...' : 'Save Changes'}
              </Button>
              {environmentId && (
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
                  {setupScriptResult.overall_success ? (
                    <>
                      <CheckCircle className="h-4 w-4 text-green-500" />
                      <span className="text-green-600">
                        All commands succeeded
                      </span>
                    </>
                  ) : (
                    <>
                      <XCircle className="h-4 w-4 text-red-500" />
                      <span className="text-red-600">Some commands failed</span>
                    </>
                  )}
                  {setupScriptResult.total_duration_ms && (
                    <span className="text-sm text-muted-foreground ml-auto">
                      Duration: {setupScriptResult.total_duration_ms}ms
                    </span>
                  )}
                </div>
              )}
            </SheetDescription>
          </SheetHeader>
          <ScrollArea className="h-[calc(100vh-10rem)] mt-6">
            {setupScriptResult && (
              <div className="space-y-4">
                {setupScriptResult.commands.map(
                  (command: SetupScriptCommand, index: number) => (
                    <div
                      key={`command-${index}-${command.command.slice(0, 20)}`}
                      className={`rounded-lg border ${
                        command.success
                          ? 'border-green-200 dark:border-green-800'
                          : 'border-red-200 dark:border-red-800'
                      } p-4 space-y-2`}
                    >
                      <div className="flex items-start justify-between gap-2">
                        <code className="text-sm font-mono bg-muted px-2 py-1 rounded flex-1">
                          $ {command.command}
                        </code>
                        <div className="flex items-center gap-2">
                          {command.success ? (
                            <CheckCircle className="h-4 w-4 text-green-500 flex-shrink-0" />
                          ) : (
                            <XCircle className="h-4 w-4 text-red-500 flex-shrink-0" />
                          )}
                          <span className="text-xs text-muted-foreground">
                            Exit code: {command.exit_code}
                          </span>
                        </div>
                      </div>
                      {command.output && (
                        <div className="space-y-1">
                          <div className="text-xs font-medium text-muted-foreground">
                            Output:
                          </div>
                          <pre className="text-xs bg-muted p-3 rounded overflow-auto whitespace-pre-wrap max-h-64">
                            {command.output}
                          </pre>
                        </div>
                      )}
                      {command.error && (
                        <div className="space-y-1">
                          <div className="text-xs font-medium text-red-600">
                            Error:
                          </div>
                          <pre className="text-xs bg-red-50 dark:bg-red-900/10 text-red-900 dark:text-red-100 p-3 rounded overflow-auto whitespace-pre-wrap max-h-64">
                            {command.error}
                          </pre>
                        </div>
                      )}
                    </div>
                  ),
                )}
              </div>
            )}
          </ScrollArea>
        </SheetContent>
      </Sheet>
    </>
  );
}
