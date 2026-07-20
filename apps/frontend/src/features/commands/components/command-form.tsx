import type {
  CommandResponse,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateCommand, useUpdateCommand } from '@/features/commands/hooks/use-commands'
import { ConfigurationInput, ConfigurationItem } from '@/lib/components/configuration'
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
type CommandVariant = 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'

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
  variant?: CommandVariant
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

function formDataFromCommand(command: CommandResponse): CommandFormData {
  return {
    name: command.name,
    description: command.description,
    prompt: command.prompt,
    conditions: command.conditions || {},
    icon: command.icon || '',
    variant: (command.variant as CommandVariant) || 'outline',
    active: command.active ?? true,
    willNotResume: command.willNotResume ?? false,
    agentMode: (command.agentMode as CommandFormData['agentMode']) || 'edit',
  }
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
        onCheckedChange={(value) => onCheckedChange(value === true)}
      />
      <Label htmlFor={id} className="text-sm">
        {label}
      </Label>
    </div>
  )
}

interface CommandFormProps {
  /** The command being edited. Omit for create mode. */
  command?: CommandResponse
  /** Called with the saved command after a successful create/update. */
  onSaved?: (command: CommandResponse) => void
}

/**
 * Editable form for a single command. In edit mode (`command` provided) the name
 * field is read-only. Manages its own draft state — mount `key`ed by command name
 * to reset when switching commands.
 */
export function CommandForm({ command, onSaved }: CommandFormProps) {
  const isEdit = command !== undefined
  const createCommand = useCreateCommand()
  const updateCommand = useUpdateCommand()
  const [formData, setFormData] = useState<CommandFormData>(
    command ? formDataFromCommand(command) : initialFormData,
  )

  const isPending = createCommand.isPending || updateCommand.isPending

  const updateCondition = (key: string, value: boolean) => {
    setFormData((prev) => ({
      ...prev,
      conditions: {
        ...prev.conditions,
        [key]: value || undefined,
      },
    }))
  }

  const handleSave = () => {
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

    const request = buildCommandRequest(formData)

    if (isEdit) {
      updateCommand.mutate(
        { commandName: command.name, request },
        {
          onSuccess: (saved) => {
            toast.success('Command updated successfully')
            onSaved?.(saved)
          },
          onError: (err) =>
            toast.error('Failed to update command', { description: getErrorMessage(err) }),
        },
      )
    } else {
      createCommand.mutate(request as CreateCommandRequest, {
        onSuccess: (saved) => {
          toast.success('Command created successfully')
          onSaved?.(saved)
        },
        onError: (err) =>
          toast.error('Failed to create command', { description: getErrorMessage(err) }),
      })
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{isEdit ? 'Configuration' : 'New Command'}</CardTitle>
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
              disabled={isEdit}
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
              onChange={(e) => setFormData((prev) => ({ ...prev, description: e.target.value }))}
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
              onCheckedChange={(value) => updateCondition('requiresUncommittedChanges', value)}
            />
            <ConditionCheckbox
              id="requiresCommittedChanges"
              label="Committed changes"
              checked={formData.conditions.requiresCommittedChanges}
              onCheckedChange={(value) => updateCondition('requiresCommittedChanges', value)}
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
              onCheckedChange={(value) => updateCondition('requiresLastIterationError', value)}
            />
            <ConditionCheckbox
              id="requiresLastIterationSuccess"
              label="Last iteration successful"
              checked={formData.conditions.requiresLastIterationSuccess}
              onCheckedChange={(value) => updateCondition('requiresLastIterationSuccess', value)}
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
                  setFormData((prev) => ({ ...prev, variant: value as CommandVariant }))
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
                  setFormData((prev) => ({ ...prev, agentMode: value as 'edit' | 'plan' }))
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
                onCheckedChange={(checked) => setFormData((prev) => ({ ...prev, active: checked }))}
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
          <Button onClick={handleSave} disabled={isPending}>
            {isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            {isEdit ? 'Save changes' : 'Create command'}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
