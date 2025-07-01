import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/shared/types/command'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useId, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

interface CommandFormProps {
  command?: Command
  onSubmit: (data: CreateCommandRequest | UpdateCommandRequest) => void
  isSubmitting?: boolean
}

type FormData = {
  name: string
  description: string
  prompt: string
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
  })

  // Update form data when command prop changes
  useEffect(() => {
    if (command) {
      setFormData({
        name: command.name,
        description: command.description,
        prompt: command.prompt,
      })
    }
  }, [command])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (!formData.name.trim()) {
      toast.error('Please provide a command name')
      return
    }

    if (!formData.description.trim()) {
      toast.error('Please provide a command description')
      return
    }

    if (!formData.prompt.trim()) {
      toast.error('Please provide a command prompt')
      return
    }

    // Validate command name format
    if (!/^[a-zA-Z0-9_-]+$/.test(formData.name)) {
      toast.error(
        'Command name must contain only alphanumeric characters, hyphens, and underscores',
      )
      return
    }

    onSubmit({
      name: formData.name.trim(),
      description: formData.description.trim(),
      prompt: formData.prompt.trim(),
    } as CreateCommandRequest)
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
              className={isEdit ? 'bg-muted' : ''}
            />
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
            />
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
              className="font-mono text-sm"
            />
            <p className="text-sm text-muted-foreground">
              The exact prompt that will be sent to the code agent when this command is selected
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
