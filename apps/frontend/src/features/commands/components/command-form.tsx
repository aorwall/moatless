import { CreateCommandSchema, UpdateCommandSchema } from '@moatless/types'
import type {
  Command,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import * as LucideIcons from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Checkbox } from '@/lib/components/ui/checkbox'
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
import { useZodForm } from '@/lib/hooks/use-zod-form'

interface CommandFormProps {
  command?: Command
  onSubmit: (data: CreateCommandRequest | UpdateCommandRequest) => void
  isSubmitting?: boolean
}

type FormData = {
  name: string
  description: string
  prompt: string
  conditions?: {
    requiresUncommittedChanges?: boolean
    requiresCommittedChanges?: boolean
    requiresBehindMain?: boolean
    requiresAheadOfMain?: boolean
    requiresDiverged?: boolean
    requiresTaskNotRunning?: boolean
    requiresLastIterationError?: boolean
    requiresLastIterationSuccess?: boolean
    allowedGitSyncStatuses?: Array<
      'synced' | 'ahead' | 'behind' | 'diverged' | 'uncommitted_changes' | 'error'
    >
    allowedTaskStatuses?: Array<'open' | 'running' | 'error' | 'merged' | 'closed'>
  }
  icon?: string
  variant?: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'
}

export function CommandForm({ command, onSubmit, isSubmitting }: CommandFormProps) {
  const navigate = useNavigate()
  const isEdit = !!command

  const nameId = useId()
  const descriptionId = useId()
  const promptId = useId()

  const [formData, setFormData] = useState<FormData>({
    name: command?.name || '',
    description: command?.description || '',
    prompt: command?.prompt || '',
    conditions: command?.conditions || {},
    icon: command?.icon || '',
    variant: command?.variant || 'default',
  })

  // Use the appropriate schema based on whether we're editing or creating
  const schema = isEdit ? UpdateCommandSchema : CreateCommandSchema
  const { errors, validate, clearErrors } = useZodForm({ schema })

  // Update form data when command prop changes
  useEffect(() => {
    if (command) {
      setFormData({
        name: command.name,
        description: command.description,
        prompt: command.prompt,
        conditions: command.conditions || {},
        icon: command.icon || '',
        variant: command.variant || 'default',
      })
    }
  }, [command])

  // Clear errors when form data changes
  useEffect(() => {
    clearErrors()
  }, [clearErrors])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    // Prepare data for validation
    const submitData: CreateCommandRequest | UpdateCommandRequest = {
      name: formData.name.trim(),
      description: formData.description.trim(),
      prompt: formData.prompt.trim(),
    }

    // Only include optional fields if they have values
    if (formData.conditions && Object.keys(formData.conditions).length > 0) {
      submitData.conditions = formData.conditions
    }
    if (formData.icon) {
      submitData.icon = formData.icon
    }
    if (formData.variant && formData.variant !== 'default') {
      submitData.variant = formData.variant
    }

    // Validate using schema
    const result = validate(submitData)

    if (!result.success) {
      // Show first error as a toast
      const firstError = result.error.errors[0]
      if (firstError) {
        toast.error(firstError.message)
      }
      return
    }

    onSubmit(result.data)
  }

  const handleCancel = () => {
    navigate({ to: '/commands' })
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>{isEdit ? 'Edit Command' : 'Create Command'}</CardTitle>
          <CardDescription>
            {isEdit
              ? 'Update the command details below'
              : 'Create a new command that can be sent to the code agent'}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor={nameId}>Name</Label>
            <Input
              id={nameId}
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
              placeholder="e.g., commit, test, build"
              disabled={isEdit}
              className={isEdit ? 'bg-muted' : errors.name ? 'border-destructive' : ''}
            />
            {errors.name && <p className="text-sm text-destructive">{errors.name}</p>}
            <p className="text-sm text-muted-foreground">
              A unique identifier for the command. Use only letters, numbers, hyphens, and
              underscores.
              {isEdit && ' Cannot be changed after creation.'}
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor={descriptionId}>Description</Label>
            <Input
              id={descriptionId}
              value={formData.description}
              onChange={(e) => setFormData({ ...formData, description: e.target.value })}
              placeholder="e.g., Commit the changes to the repository"
              className={errors.description ? 'border-destructive' : ''}
            />
            {errors.description && <p className="text-sm text-destructive">{errors.description}</p>}
            <p className="text-sm text-muted-foreground">
              A brief description of what this command does
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor={promptId}>Prompt</Label>
            <Textarea
              id={promptId}
              value={formData.prompt}
              onChange={(e) => setFormData({ ...formData, prompt: e.target.value })}
              placeholder="e.g., commit the relevant changes"
              rows={4}
              className={`font-mono text-sm ${errors.prompt ? 'border-destructive' : ''}`}
            />
            {errors.prompt && <p className="text-sm text-destructive">{errors.prompt}</p>}
            <p className="text-sm text-muted-foreground">
              The exact prompt that will be sent to the code agent when this command is selected
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Conditions</CardTitle>
          <CardDescription>Define when this command should be available</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div>
            <Label className="text-base font-medium mb-3 block">Git Status Conditions</Label>
            <div className="space-y-3">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresUncommittedChanges"
                  checked={formData.conditions?.requiresUncommittedChanges || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresUncommittedChanges: checked as boolean,
                      },
                    })
                  }
                />
                <Label
                  htmlFor="requiresUncommittedChanges"
                  className="text-sm font-normal cursor-pointer"
                >
                  Requires uncommitted changes
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresCommittedChanges"
                  checked={formData.conditions?.requiresCommittedChanges || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresCommittedChanges: checked as boolean,
                      },
                    })
                  }
                />
                <Label
                  htmlFor="requiresCommittedChanges"
                  className="text-sm font-normal cursor-pointer"
                >
                  Requires committed changes
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresBehindMain"
                  checked={formData.conditions?.requiresBehindMain || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresBehindMain: checked as boolean,
                      },
                    })
                  }
                />
                <Label htmlFor="requiresBehindMain" className="text-sm font-normal cursor-pointer">
                  Requires branch behind main
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresAheadOfMain"
                  checked={formData.conditions?.requiresAheadOfMain || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresAheadOfMain: checked as boolean,
                      },
                    })
                  }
                />
                <Label htmlFor="requiresAheadOfMain" className="text-sm font-normal cursor-pointer">
                  Requires branch ahead of main
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresDiverged"
                  checked={formData.conditions?.requiresDiverged || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresDiverged: checked as boolean,
                      },
                    })
                  }
                />
                <Label htmlFor="requiresDiverged" className="text-sm font-normal cursor-pointer">
                  Requires branch diverged from main
                </Label>
              </div>
            </div>
          </div>

          <div>
            <Label className="text-base font-medium mb-3 block">Task Status Conditions</Label>
            <div className="space-y-3">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresTaskNotRunning"
                  checked={formData.conditions?.requiresTaskNotRunning || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresTaskNotRunning: checked as boolean,
                      },
                    })
                  }
                />
                <Label
                  htmlFor="requiresTaskNotRunning"
                  className="text-sm font-normal cursor-pointer"
                >
                  Requires task not running
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresLastIterationError"
                  checked={formData.conditions?.requiresLastIterationError || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresLastIterationError: checked as boolean,
                      },
                    })
                  }
                />
                <Label
                  htmlFor="requiresLastIterationError"
                  className="text-sm font-normal cursor-pointer"
                >
                  Requires last iteration had error
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="requiresLastIterationSuccess"
                  checked={formData.conditions?.requiresLastIterationSuccess || false}
                  onCheckedChange={(checked) =>
                    setFormData({
                      ...formData,
                      conditions: {
                        ...formData.conditions,
                        requiresLastIterationSuccess: checked as boolean,
                      },
                    })
                  }
                />
                <Label
                  htmlFor="requiresLastIterationSuccess"
                  className="text-sm font-normal cursor-pointer"
                >
                  Requires last iteration was successful
                </Label>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>UI Customization</CardTitle>
          <CardDescription>Customize how this command appears in the interface</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="icon">Icon</Label>
            <Select
              value={formData.icon || 'none'}
              onValueChange={(value) =>
                setFormData({ ...formData, icon: value === 'none' ? '' : value })
              }
            >
              <SelectTrigger id="icon">
                <SelectValue placeholder="Select an icon (optional)">
                  {formData.icon && (
                    <div className="flex items-center gap-2">
                      {(() => {
                        const IconComponent = (LucideIcons as any)[formData.icon]
                        return IconComponent ? <IconComponent className="h-4 w-4" /> : null
                      })()}
                      <span>{formData.icon}</span>
                    </div>
                  )}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">No icon</SelectItem>
                <SelectItem value="GitCommit">
                  <div className="flex items-center gap-2">
                    <LucideIcons.GitCommit className="h-4 w-4" />
                    <span>GitCommit</span>
                  </div>
                </SelectItem>
                <SelectItem value="GitBranch">
                  <div className="flex items-center gap-2">
                    <LucideIcons.GitBranch className="h-4 w-4" />
                    <span>GitBranch</span>
                  </div>
                </SelectItem>
                <SelectItem value="GitMerge">
                  <div className="flex items-center gap-2">
                    <LucideIcons.GitMerge className="h-4 w-4" />
                    <span>GitMerge</span>
                  </div>
                </SelectItem>
                <SelectItem value="GitPullRequest">
                  <div className="flex items-center gap-2">
                    <LucideIcons.GitPullRequest className="h-4 w-4" />
                    <span>GitPullRequest</span>
                  </div>
                </SelectItem>
                <SelectItem value="Play">
                  <div className="flex items-center gap-2">
                    <LucideIcons.Play className="h-4 w-4" />
                    <span>Play</span>
                  </div>
                </SelectItem>
                <SelectItem value="TestTube">
                  <div className="flex items-center gap-2">
                    <LucideIcons.TestTube className="h-4 w-4" />
                    <span>TestTube</span>
                  </div>
                </SelectItem>
                <SelectItem value="Hammer">
                  <div className="flex items-center gap-2">
                    <LucideIcons.Hammer className="h-4 w-4" />
                    <span>Hammer</span>
                  </div>
                </SelectItem>
                <SelectItem value="Package">
                  <div className="flex items-center gap-2">
                    <LucideIcons.Package className="h-4 w-4" />
                    <span>Package</span>
                  </div>
                </SelectItem>
                <SelectItem value="Bug">
                  <div className="flex items-center gap-2">
                    <LucideIcons.Bug className="h-4 w-4" />
                    <span>Bug</span>
                  </div>
                </SelectItem>
                <SelectItem value="Zap">
                  <div className="flex items-center gap-2">
                    <LucideIcons.Zap className="h-4 w-4" />
                    <span>Zap</span>
                  </div>
                </SelectItem>
              </SelectContent>
            </Select>
            <p className="text-sm text-muted-foreground">
              Choose an icon to display next to the command button
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="variant">Button Style</Label>
            <Select
              value={formData.variant || 'default'}
              onValueChange={(value) =>
                setFormData({
                  ...formData,
                  variant: value as 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive',
                })
              }
            >
              <SelectTrigger id="variant">
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
            <p className="text-sm text-muted-foreground">
              Choose the visual style of the command button
            </p>
          </div>
        </CardContent>
      </Card>

      <div className="flex gap-4">
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : isEdit ? 'Update Command' : 'Create Command'}
        </Button>
        <Button type="button" variant="outline" onClick={handleCancel} disabled={isSubmitting}>
          Cancel
        </Button>
      </div>
    </form>
  )
}
