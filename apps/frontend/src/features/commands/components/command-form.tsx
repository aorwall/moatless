import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/types'
import { CreateCommandSchema, UpdateCommandSchema } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { CommandBasicFields } from './command-basic-fields'
import { CommandUICustomization } from './command-ui-customization'
import { GitStatusConditions } from './git-status-conditions'
import { TaskStatusConditions } from './task-status-conditions'

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
    allowedGitSyncStatuses?: (
      | 'synced'
      | 'ahead'
      | 'behind'
      | 'diverged'
      | 'uncommitted_changes'
      | 'error'
    )[]
    allowedTaskStatuses?: ('open' | 'running' | 'error' | 'merged' | 'closed')[]
  }
  icon?: string
  variant?: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'
}

export function CommandForm({ command, onSubmit, isSubmitting }: CommandFormProps) {
  const navigate = useNavigate()
  const isEdit = command != null

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
      const firstError = result.error.issues[0]
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
          <CommandBasicFields
            name={formData.name}
            description={formData.description}
            prompt={formData.prompt}
            isEdit={isEdit}
            errors={errors}
            onNameChange={(name) => setFormData({ ...formData, name })}
            onDescriptionChange={(description) => setFormData({ ...formData, description })}
            onPromptChange={(prompt) => setFormData({ ...formData, prompt })}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Conditions</CardTitle>
          <CardDescription>Define when this command should be available</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <GitStatusConditions
            conditions={formData.conditions}
            onConditionsChange={(conditions) =>
              setFormData({
                ...formData,
                conditions: { ...formData.conditions, ...conditions },
              })
            }
          />

          <TaskStatusConditions
            conditions={formData.conditions}
            onConditionsChange={(conditions) =>
              setFormData({
                ...formData,
                conditions: { ...formData.conditions, ...conditions },
              })
            }
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>UI Customization</CardTitle>
          <CardDescription>Customize how this command appears in the interface</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <CommandUICustomization
            icon={formData.icon}
            variant={formData.variant}
            onIconChange={(icon) => setFormData({ ...formData, icon })}
            onVariantChange={(variant) => setFormData({ ...formData, variant })}
          />
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
