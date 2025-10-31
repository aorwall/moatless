import type { CreateRepositoryRequest } from '@moatless/types'
import { CreateRepositorySchema } from '@moatless/types'
import { Github, HardDrive } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import type { z } from 'zod'
import { useCreateRepository } from '@/features/repositories/hooks/use-repositories'

interface RepositoryFormData {
  name: string
  description: string
  path: string
  remoteUrl: string
  useGitWorktrees: boolean
  cloneOnNewTask: boolean
  systemPrompt?: string
}

type RepositoryField = keyof RepositoryFormData

import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'
import { AgentConfigurationGroup } from './agent-configuration-group'

interface RepositoryConfigCreateSectionProps {
  onCancel: () => void
  onSuccess: (repositoryId: string) => void
}

export function RepositoryConfigCreateSection({
  onCancel,
  onSuccess,
}: RepositoryConfigCreateSectionProps) {
  const createRepository = useCreateRepository()
  const { errors, validate, clearErrors } = useZodForm({
    schema: CreateRepositorySchema,
  })

  const [sourceType, setSourceType] = useState<'local' | 'github'>('local')
  const [formData, setFormData] = useState<RepositoryFormData>({
    name: '',
    description: '',
    path: '',
    remoteUrl: '',
    useGitWorktrees: false,
    cloneOnNewTask: false,
  })

  // Helper: Extract repository name from GitHub HTTPS URL
  const extractRepoNameFromUrl = (url: string): string | null => {
    // Only support GitHub HTTPS URLs
    const regex = /^https:\/\/github\.com\/[^/]+\/([^/.]+)(?:\.git)?$/
    const match = url.match(regex)
    return match?.[1] || null
  }

  // Helper: Create repository data for GitHub source
  const createGitHubRepositoryData = (
    remoteUrl: string,
  ): CreateRepositoryRequest | { error: string } => {
    if (!remoteUrl) {
      return { error: 'Please enter a GitHub repository URL' }
    }

    const repoName = extractRepoNameFromUrl(remoteUrl)

    if (!repoName) {
      return {
        error: 'Please enter a valid GitHub HTTPS URL (e.g., https://github.com/owner/repo)',
      }
    }

    return {
      name: repoName,
      remoteUrl,
      sourceType: 'github',
      useGitWorktrees: false,
      cloneOnNewTask: false,
      systemPrompt: formData.systemPrompt?.trim() || undefined,
      codeExecution: {},
    }
  }

  // Helper: Create repository data for local source
  const createLocalRepositoryData = (): CreateRepositoryRequest => {
    return {
      name: formData.name.trim(),
      description: formData.description.trim() || undefined,
      path: formData.path.trim(),
      sourceType: 'local',
      useGitWorktrees: formData.useGitWorktrees,
      cloneOnNewTask: formData.cloneOnNewTask,
      systemPrompt: formData.systemPrompt?.trim() || undefined,
      codeExecution: {},
    }
  }

  // Helper: Show validation errors
  const showValidationErrors = (result: {
    success: false
    error: z.ZodError<CreateRepositoryRequest>
  }): void => {
    const errorCount = result.error.issues.length
    if (errorCount === 1) {
      const error = result.error.issues[0]
      const fieldName = error.path[0] || 'field'
      toast.error(`Please check the ${String(fieldName)} field: ${error.message}`)
    } else {
      toast.error(`Please check the form for ${errorCount} validation errors`)
    }
  }

  const handleSubmit = () => {
    let repositoryData: CreateRepositoryRequest

    if (sourceType === 'github') {
      const result = createGitHubRepositoryData(formData.remoteUrl.trim())
      if ('error' in result) {
        toast.error(result.error)
        return
      }
      repositoryData = result
    } else {
      repositoryData = createLocalRepositoryData()
    }

    const result = validate(repositoryData)

    if (!result.success) {
      showValidationErrors(result)
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

  const handleFieldChange = <Field extends RepositoryField>(
    field: Field,
    value: RepositoryFormData[Field],
  ) => {
    setFormData((prev) => ({ ...prev, [field]: value }))
    clearErrors()
  }

  const handleSourceTypeChange = (value: string | null) => {
    if (value === 'local' || value === 'github') {
      setSourceType(value)
    }
  }

  // Handle GitHub URL change
  const handleGitHubUrlChange = (url: string) => {
    handleFieldChange('remoteUrl', url)
    // No need to extract name here since it's not visible in the UI for GitHub repos
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Repository Source">
        <ConfigurationItem
          label="Source Type"
          description="Choose between a local repository or clone from GitHub"
        >
          <ToggleGroup
            type="single"
            value={sourceType}
            onValueChange={handleSourceTypeChange}
            className="justify-start"
          >
            <ToggleGroupItem value="local" aria-label="Local repository">
              <HardDrive className="h-4 w-4 mr-2" />
              Local Repository
            </ToggleGroupItem>
            <ToggleGroupItem value="github" aria-label="GitHub repository">
              <Github className="h-4 w-4 mr-2" />
              GitHub Repository
            </ToggleGroupItem>
          </ToggleGroup>
        </ConfigurationItem>
      </ConfigurationGroup>

      {sourceType === 'local' ? (
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
            description="Absolute path to your local repository (disabled when cloning on each task)"
          >
            <ConfigurationInput
              value={formData.path}
              onChange={(e) => handleFieldChange('path', e.target.value)}
              placeholder="/Users/username/projects/my-project"
              disabled={formData.cloneOnNewTask}
              className={
                errors.path ? 'border-destructive' : formData.cloneOnNewTask ? 'bg-muted' : ''
              }
            />
            {errors.path && <p className="text-sm text-destructive mt-1">{errors.path}</p>}
          </ConfigurationItem>

          <ConfigurationItem
            label="Git Remote URL"
            description="Optional GitHub HTTPS URL (enables cloning feature)"
          >
            <ConfigurationInput
              value={formData.remoteUrl}
              onChange={(e) => handleFieldChange('remoteUrl', e.target.value)}
              placeholder="https://github.com/owner/repo"
            />
          </ConfigurationItem>

          <ConfigurationItem
            label="Git Worktrees"
            description="Use git worktrees for isolated environments (recommended for git repositories)"
          >
            <ConfigurationSwitch
              checked={formData.useGitWorktrees}
              onCheckedChange={(checked) => {
                handleFieldChange('useGitWorktrees', checked)
                // Disable cloneOnNewTask when worktrees are enabled
                if (checked) {
                  handleFieldChange('cloneOnNewTask', false)
                }
              }}
            />
          </ConfigurationItem>

          <ConfigurationItem
            label="Clone on New Task"
            description="Requires a Git remote URL. Clone the entire repository for each new task instead of using git worktrees. Provides complete isolation between tasks."
          >
            <ConfigurationSwitch
              checked={formData.cloneOnNewTask}
              disabled={!formData.remoteUrl}
              onCheckedChange={(checked) => {
                handleFieldChange('cloneOnNewTask', checked)
                // Disable worktrees when cloning is enabled
                if (checked) {
                  handleFieldChange('useGitWorktrees', false)
                }
              }}
            />
          </ConfigurationItem>
        </ConfigurationGroup>
      ) : (
        <ConfigurationGroup title="GitHub Repository">
          <ConfigurationItem
            label="Repository URL"
            description="Enter the GitHub HTTPS repository URL to clone"
          >
            <ConfigurationInput
              value={formData.remoteUrl}
              onChange={(e) => handleGitHubUrlChange(e.target.value)}
              placeholder="https://github.com/owner/repo"
              className={errors.remoteUrl ? 'border-destructive' : ''}
              autoFocus
            />
            {errors.remoteUrl && (
              <p className="text-sm text-destructive mt-1">{errors.remoteUrl}</p>
            )}
          </ConfigurationItem>
        </ConfigurationGroup>
      )}

      <AgentConfigurationGroup
        formData={formData}
        onUpdateField={(field, value) => {
          if (field === 'systemPrompt') {
            handleFieldChange('systemPrompt', value as string | undefined)
          }
        }}
      />

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
