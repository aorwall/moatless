import type { CreateRepositoryRequest } from '@moatless/types'
import { CreateRepositorySchema } from '@moatless/types'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateRepository } from '@/features/repositories/hooks/use-repositories'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'

interface RepositoryNewSectionProps {
  onCancel: () => void
  onSuccess: (repositoryId: string) => void
}

export function RepositoryNewSection({ onCancel, onSuccess }: RepositoryNewSectionProps) {
  const createRepository = useCreateRepository()
  const { errors, validate, clearErrors } = useZodForm({ schema: CreateRepositorySchema })

  const [formData, setFormData] = useState({
    name: '',
    description: '',
    path: '',
    useGitWorktrees: false,
  })

  const handleSubmit = () => {
    const repositoryData: CreateRepositoryRequest = {
      name: formData.name.trim(),
      description: formData.description.trim() || undefined,
      path: formData.path.trim(),
      useGitWorktrees: formData.useGitWorktrees,
      codeExecution: {
        type: 'local',
      },
    }

    const result = validate(repositoryData)

    if (!result.success) {
      const errorCount = result.error.issues.length
      if (errorCount === 1) {
        const error = result.error.issues[0]
        const fieldName = error.path[0] || 'field'
        toast.error(`Please check the ${String(fieldName)} field: ${error.message}`)
      } else {
        toast.error(`Please check the form for ${errorCount} validation errors`)
      }
      return
    }

    createRepository.mutate(result.data, {
      onSuccess: (data) => {
        toast.success('Repository created successfully')
        onSuccess(data.id)
      },
      onError: (error) => {
        toast.error('Failed to create repository', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleFieldChange = (field: keyof typeof formData, value: any) => {
    setFormData((prev) => ({ ...prev, [field]: value }))
    clearErrors()
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Repository Details">
        <ConfigurationItem label="Repository Name" description="Display name for your repository">
          <ConfigurationInput
            value={formData.name}
            onChange={(e) => handleFieldChange('name', e.target.value)}
            placeholder="My Project Repository"
            className={errors.name ? 'border-destructive' : ''}
          />
          {errors.name && <p className="text-sm text-destructive mt-1">{errors.name}</p>}
        </ConfigurationItem>

        <ConfigurationItem
          label="Description"
          description="Optional description of this repository"
        >
          <ConfigurationTextarea
            value={formData.description}
            onChange={(e) => handleFieldChange('description', e.target.value)}
            placeholder="Brief description of your repository"
            rows={3}
          />
        </ConfigurationItem>

        <ConfigurationItem
          label="Repository Path"
          description="Absolute path to your local repository"
        >
          <ConfigurationInput
            value={formData.path}
            onChange={(e) => handleFieldChange('path', e.target.value)}
            placeholder="/Users/username/projects/my-project"
            className={errors.path ? 'border-destructive' : ''}
          />
          {errors.path && <p className="text-sm text-destructive mt-1">{errors.path}</p>}
        </ConfigurationItem>

        <ConfigurationItem
          label="Git Worktrees"
          description="Use git worktrees for isolated environments (recommended for git repositories)"
        >
          <ConfigurationSwitch
            checked={formData.useGitWorktrees}
            onCheckedChange={(checked) => handleFieldChange('useGitWorktrees', checked)}
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      <div className="flex justify-end gap-3 pt-6 border-t">
        <Button
          type="button"
          variant="outline"
          onClick={onCancel}
          disabled={createRepository.isPending}
        >
          Cancel
        </Button>
        <Button type="button" onClick={handleSubmit} disabled={createRepository.isPending}>
          {createRepository.isPending ? 'Creating...' : 'Create Repository'}
        </Button>
      </div>
    </ConfigurationSection>
  )
}
