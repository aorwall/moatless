import type {
  CommandResponse,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/api-client'
import { Loader2, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useCommandsList,
  useCreateCommand,
  useDeleteCommand,
  useUpdateCommand,
} from '@/features/commands/hooks/use-commands'
import { getLucideIcon } from '@/features/commands/utils/lucide-icon'
import { ConfigurationInput, ConfigurationItem } from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Switch } from '@/lib/components/ui/switch'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

type GitSyncStatus = 'synced' | 'ahead' | 'behind' | 'diverged' | 'uncommitted_changes' | 'error'
type TaskStatus = 'open' | 'error' | 'closed'

interface CommandFormData {
  name: string
  description: string
  prompt: string
  conditions: {
    requiresUncommittedChanges?: boolean
    requiresCommittedChanges?: boolean
    requiresBehindMain?: boolean
    requiresAheadOfMain?: boolean
    requiresDiverged?: boolean
    requiresTaskNotRunning?: boolean
    requiresLastIterationError?: boolean
    requiresLastIterationSuccess?: boolean
    allowedGitSyncStatuses?: GitSyncStatus[]
    allowedTaskStatuses?: TaskStatus[]
  }
  icon?: string
  variant?: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'
  active?: boolean
  willNotResume?: boolean
  agentMode?: 'edit' | 'plan'
}

const initialFormData: CommandFormData = {
  name: '',
  description: '',
  prompt: '',
  conditions: {},
  icon: '',
  variant: 'outline',
  active: true,
  willNotResume: false,
  agentMode: 'edit',
}

function buildCommandRequest(
  formData: CommandFormData,
): CreateCommandRequest | UpdateCommandRequest {
  const conditions = Object.keys(formData.conditions).length > 0 ? formData.conditions : undefined
  return {
    name: formData.name,
    description: formData.description,
    prompt: formData.prompt,
    ...(conditions !== undefined && { conditions }),
    ...(formData.icon && { icon: formData.icon }),
    ...(formData.variant !== undefined && { variant: formData.variant }),
    ...(formData.active !== undefined && { active: formData.active }),
    ...(formData.willNotResume !== undefined && { willNotResume: formData.willNotResume }),
    ...(formData.agentMode !== undefined && { agentMode: formData.agentMode }),
  }
}

function ConditionCheckbox({
  id,
  label,
  checked,
  onCheckedChange,
}: {
  id: string
  label: string
  checked: boolean | undefined
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center space-x-2">
      <Checkbox
        id={id}
        checked={checked ?? false}
        onCheckedChange={(checked) => onCheckedChange(checked === true)}
      />
      <Label htmlFor={id} className="text-sm">
        {label}
      </Label>
    </div>
  )
}

export function CommandsPage() {
  const { data: commands, isLoading, error } = useCommandsList()
  const createCommand = useCreateCommand()
  const updateCommand = useUpdateCommand()
  const deleteCommand = useDeleteCommand()

  const [editingCommand, setEditingCommand] = useState<string | null>(null)
  const [formData, setFormData] = useState<CommandFormData>(initialFormData)

  const handleAddCommand = () => {
    setEditingCommand('__new__')
    setFormData(initialFormData)
  }

  const handleEditCommand = (command: CommandResponse) => {
    setEditingCommand(command.name)
    setFormData({
      name: command.name,
      description: command.description,
      prompt: command.prompt,
      conditions: command.conditions || {},
      icon: command.icon || '',
      variant: (command.variant as CommandFormData['variant']) || 'outline',
      active: command.active ?? true,
      willNotResume: command.willNotResume ?? false,
      agentMode: (command.agentMode as CommandFormData['agentMode']) || 'edit',
    })
  }

  const handleSaveCommand = () => {
    if (editingCommand === null) {
      return
    }

    if (!formData.name.trim()) {
      toast.error('Command name is required')
      return
    }
    if (!formData.description.trim()) {
      toast.error('Description is required')
      return
    }
    if (!formData.prompt.trim()) {
      toast.error('Prompt is required')
      return
    }

    const commandData = buildCommandRequest(formData)

    if (editingCommand === '__new__') {
      createCommand.mutate(commandData as CreateCommandRequest, {
        onSuccess: () => {
          setEditingCommand(null)
          setFormData(initialFormData)
          toast.success('Command created successfully')
        },
        onError: (err) => {
          toast.error('Failed to create command', {
            description: getErrorMessage(err),
          })
        },
      })
    } else {
      updateCommand.mutate(
        { commandName: editingCommand, request: commandData },
        {
          onSuccess: () => {
            setEditingCommand(null)
            setFormData(initialFormData)
            toast.success('Command updated successfully')
          },
          onError: (err) => {
            toast.error('Failed to update command', {
              description: getErrorMessage(err),
            })
          },
        },
      )
    }
  }

  const handleDeleteCommand = (commandName: string) => {
    deleteCommand.mutate(commandName, {
      onSuccess: () => {
        toast.success('Command deleted successfully')
      },
      onError: (err) => {
        toast.error('Failed to delete command', {
          description: getErrorMessage(err),
        })
      },
    })
  }

  const handleCancelEdit = () => {
    setEditingCommand(null)
    setFormData(initialFormData)
  }

  const updateCondition = (key: string, value: boolean) => {
    setFormData((prev) => ({
      ...prev,
      conditions: {
        ...prev.conditions,
        [key]: value || undefined,
      },
    }))
  }

  return (
    <AdminPage
      title="Commands"
      description="Configure quick action commands for tasks"
      actions={
        <Button
          onClick={handleAddCommand}
          size="sm"
          variant="outline"
          disabled={isLoading || !!error}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Command
        </Button>
      }
    >
      {isLoading ? (
        <div className="flex items-center justify-center h-32">
          <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
        </div>
      ) : error ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>Failed to load commands. Please try again later.</AlertDescription>
        </Alert>
      ) : (
        <div className="space-y-4">
          {/* List of configured commands */}
          <div className="space-y-2">
            {(!commands || commands.length === 0) && !editingCommand && (
              <p className="text-sm text-muted-foreground">No commands configured</p>
            )}

            {commands?.map((command) => {
              if (editingCommand === command.name) {
                return null
              }

              const IconComponent = command.icon ? getLucideIcon(command.icon) : null

              return (
                <Card key={command.name}>
                  <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                    <CardTitle className="text-sm font-medium flex items-center gap-2">
                      {IconComponent && <IconComponent className="h-4 w-4" />}
                      {command.name}
                    </CardTitle>
                    <div className="flex gap-2">
                      <Button
                        onClick={() => handleEditCommand(command)}
                        size="sm"
                        variant="outline"
                      >
                        Edit
                      </Button>
                      <Button
                        onClick={() => handleDeleteCommand(command.name)}
                        size="sm"
                        variant="destructive"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </CardHeader>
                  <CardContent>
                    <p className="text-sm text-muted-foreground">{command.description}</p>
                    {command.active === false && (
                      <span className="text-xs text-yellow-600 dark:text-yellow-400">Inactive</span>
                    )}
                  </CardContent>
                </Card>
              )
            })}
          </div>

          {/* Command edit form */}
          {editingCommand && (
            <Card>
              <CardHeader>
                <CardTitle>
                  {editingCommand === '__new__' ? 'Add New Command' : `Edit ${editingCommand}`}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                {/* Basic Fields */}
                <div className="space-y-4">
                  <h4 className="text-sm font-medium">Basic Information</h4>

                  <ConfigurationItem label="Name" description="Unique identifier for this command">
                    <ConfigurationInput
                      type="text"
                      placeholder="my-command"
                      value={formData.name}
                      onChange={(e) => setFormData((prev) => ({ ...prev, name: e.target.value }))}
                      disabled={editingCommand !== '__new__'}
                    />
                  </ConfigurationItem>

                  <ConfigurationItem
                    label="Description"
                    description="Brief description shown on the button"
                  >
                    <ConfigurationInput
                      type="text"
                      placeholder="What this command does"
                      value={formData.description}
                      onChange={(e) =>
                        setFormData((prev) => ({ ...prev, description: e.target.value }))
                      }
                    />
                  </ConfigurationItem>

                  <ConfigurationItem label="Prompt" description="The message sent to the agent">
                    <Textarea
                      placeholder="Enter the prompt for the agent..."
                      value={formData.prompt}
                      onChange={(e) => setFormData((prev) => ({ ...prev, prompt: e.target.value }))}
                      rows={3}
                    />
                  </ConfigurationItem>
                </div>

                {/* Git Conditions */}
                <div className="space-y-3">
                  <h4 className="text-sm font-medium">Git Conditions</h4>
                  <p className="text-xs text-muted-foreground">
                    Command appears only when these git conditions are met
                  </p>

                  <div className="grid grid-cols-2 gap-3">
                    <ConditionCheckbox
                      id="requiresUncommittedChanges"
                      label="Uncommitted changes"
                      checked={formData.conditions.requiresUncommittedChanges}
                      onCheckedChange={(value) =>
                        updateCondition('requiresUncommittedChanges', value)
                      }
                    />

                    <ConditionCheckbox
                      id="requiresCommittedChanges"
                      label="Committed changes"
                      checked={formData.conditions.requiresCommittedChanges}
                      onCheckedChange={(value) =>
                        updateCondition('requiresCommittedChanges', value)
                      }
                    />

                    <ConditionCheckbox
                      id="requiresAheadOfMain"
                      label="Ahead of main"
                      checked={formData.conditions.requiresAheadOfMain}
                      onCheckedChange={(value) => updateCondition('requiresAheadOfMain', value)}
                    />

                    <ConditionCheckbox
                      id="requiresBehindMain"
                      label="Behind main"
                      checked={formData.conditions.requiresBehindMain}
                      onCheckedChange={(value) => updateCondition('requiresBehindMain', value)}
                    />

                    <ConditionCheckbox
                      id="requiresDiverged"
                      label="Diverged from main"
                      checked={formData.conditions.requiresDiverged}
                      onCheckedChange={(value) => updateCondition('requiresDiverged', value)}
                    />
                  </div>
                </div>

                {/* Task Conditions */}
                <div className="space-y-3">
                  <h4 className="text-sm font-medium">Task Conditions</h4>
                  <p className="text-xs text-muted-foreground">
                    Command appears only when these task conditions are met
                  </p>

                  <div className="grid grid-cols-2 gap-3">
                    <ConditionCheckbox
                      id="requiresTaskNotRunning"
                      label="Task not running"
                      checked={formData.conditions.requiresTaskNotRunning}
                      onCheckedChange={(value) => updateCondition('requiresTaskNotRunning', value)}
                    />

                    <ConditionCheckbox
                      id="requiresLastIterationError"
                      label="Last iteration had error"
                      checked={formData.conditions.requiresLastIterationError}
                      onCheckedChange={(value) =>
                        updateCondition('requiresLastIterationError', value)
                      }
                    />

                    <ConditionCheckbox
                      id="requiresLastIterationSuccess"
                      label="Last iteration successful"
                      checked={formData.conditions.requiresLastIterationSuccess}
                      onCheckedChange={(value) =>
                        updateCondition('requiresLastIterationSuccess', value)
                      }
                    />
                  </div>
                </div>

                {/* UI Customization */}
                <div className="space-y-4">
                  <h4 className="text-sm font-medium">Appearance</h4>

                  <div className="grid grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <Label>Icon</Label>
                      <Select
                        value={formData.icon || 'none'}
                        onValueChange={(value) =>
                          setFormData((prev) => ({ ...prev, icon: value === 'none' ? '' : value }))
                        }
                      >
                        <SelectTrigger>
                          <SelectValue placeholder="Select icon" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="none">No icon</SelectItem>
                          <SelectItem value="GitCommit">GitCommit</SelectItem>
                          <SelectItem value="GitBranch">GitBranch</SelectItem>
                          <SelectItem value="GitMerge">GitMerge</SelectItem>
                          <SelectItem value="GitPullRequest">GitPullRequest</SelectItem>
                          <SelectItem value="Play">Play</SelectItem>
                          <SelectItem value="FileText">FileText</SelectItem>
                          <SelectItem value="TestTube">TestTube</SelectItem>
                          <SelectItem value="Hammer">Hammer</SelectItem>
                          <SelectItem value="Package">Package</SelectItem>
                          <SelectItem value="Bug">Bug</SelectItem>
                          <SelectItem value="Zap">Zap</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    <div className="space-y-2">
                      <Label>Button Style</Label>
                      <Select
                        value={formData.variant || 'outline'}
                        onValueChange={(value) =>
                          setFormData((prev) => ({
                            ...prev,
                            variant: value as
                              | 'default'
                              | 'outline'
                              | 'secondary'
                              | 'ghost'
                              | 'destructive',
                          }))
                        }
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="default">Default</SelectItem>
                          <SelectItem value="outline">Outline</SelectItem>
                          <SelectItem value="secondary">Secondary</SelectItem>
                          <SelectItem value="ghost">Ghost</SelectItem>
                          <SelectItem value="destructive">Destructive</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <Label>Agent Mode</Label>
                      <Select
                        value={formData.agentMode || 'edit'}
                        onValueChange={(value) =>
                          setFormData((prev) => ({
                            ...prev,
                            agentMode: value as 'edit' | 'plan',
                          }))
                        }
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="edit">Edit</SelectItem>
                          <SelectItem value="plan">Plan</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </div>

                  <div className="flex flex-col gap-3">
                    <div className="flex items-center justify-between">
                      <div>
                        <Label>Active</Label>
                        <p className="text-xs text-muted-foreground">Show this command to users</p>
                      </div>
                      <Switch
                        checked={formData.active ?? true}
                        onCheckedChange={(checked) =>
                          setFormData((prev) => ({ ...prev, active: checked }))
                        }
                      />
                    </div>

                    <div className="flex items-center justify-between">
                      <div>
                        <Label>Will Not Resume</Label>
                        <p className="text-xs text-muted-foreground">
                          Conversation won't auto-resume after this command
                        </p>
                      </div>
                      <Switch
                        checked={formData.willNotResume ?? false}
                        onCheckedChange={(checked) =>
                          setFormData((prev) => ({ ...prev, willNotResume: checked }))
                        }
                      />
                    </div>
                  </div>
                </div>

                <div className="flex justify-end gap-2 pt-4">
                  <Button onClick={handleCancelEdit} variant="outline">
                    Cancel
                  </Button>
                  <Button
                    onClick={handleSaveCommand}
                    disabled={createCommand.isPending || updateCommand.isPending}
                  >
                    {(createCommand.isPending || updateCommand.isPending) && (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    )}
                    Save Command
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </AdminPage>
  )
}
