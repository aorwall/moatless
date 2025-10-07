import { useId } from 'react'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

interface CommandBasicFieldsProps {
  name: string
  description: string
  prompt: string
  isEdit: boolean
  errors: {
    name?: string
    description?: string
    prompt?: string
  }
  onNameChange: (name: string) => void
  onDescriptionChange: (description: string) => void
  onPromptChange: (prompt: string) => void
}

export function CommandBasicFields({
  name,
  description,
  prompt,
  isEdit,
  errors,
  onNameChange,
  onDescriptionChange,
  onPromptChange,
}: CommandBasicFieldsProps) {
  const nameId = useId()
  const descriptionId = useId()
  const promptId = useId()

  return (
    <>
      <div className="space-y-2">
        <Label htmlFor={nameId}>Name</Label>
        <Input
          id={nameId}
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder="e.g., commit, test, build"
          disabled={isEdit}
          className={isEdit ? 'bg-muted' : errors.name ? 'border-destructive' : ''}
        />
        {errors.name && <p className="text-sm text-destructive">{errors.name}</p>}
        <p className="text-sm text-muted-foreground">
          A unique identifier for the command. Use only letters, numbers, hyphens, and underscores.
          {isEdit && ' Cannot be changed after creation.'}
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor={descriptionId}>Description</Label>
        <Input
          id={descriptionId}
          value={description}
          onChange={(e) => onDescriptionChange(e.target.value)}
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
          value={prompt}
          onChange={(e) => onPromptChange(e.target.value)}
          placeholder="e.g., commit the relevant changes"
          rows={4}
          className={`font-mono text-sm ${errors.prompt ? 'border-destructive' : ''}`}
        />
        {errors.prompt && <p className="text-sm text-destructive">{errors.prompt}</p>}
        <p className="text-sm text-muted-foreground">
          The exact prompt that will be sent to the code agent when this command is selected
        </p>
      </div>
    </>
  )
}
