import type { CreateRepositoryRequest } from '@moatless/types'
import { CreateRepositorySchema } from '@moatless/types'
import { Github, HardDrive } from 'lucide-react'
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
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'

interface RepositoryNewSectionProps {
  onCancel: () => void
  onSuccess: (repositoryId: string) => void
}

export function RepositoryNewSection({ onCancel, onSuccess }: RepositoryNewSectionProps) {
  const createRepository = useCreateRepository()
  const { errors, validate, clearErrors } = useZodForm({ schema: CreateRepositorySchema })

  const [sourceType, setSourceType] = useState<'local' | 'github'>('local')
  const [formData, setFormData] = useState({
    name: '',
    description: '',
    path: '',
    remoteUrl: '',
    useGitWorktrees: false,
  })

  const handleSubmit = () => {
    let repositoryData: CreateRepositoryRequest

    if (sourceType === 'github') {
      // For GitHub repos, validate URL first
      const remoteUrl = formData.remoteUrl.trim()

      if (!remoteUrl) {
        toast.error('Please enter a GitHub repository URL')
        return
      }

      // Extract name from URL for validation
      // Support both HTTPS and SSH URLs
      const httpsPattern = /(?:https?:\/\/)?(?:www\.)?github\.com\/[^/]+\/([^/.]+)(?:\.git)?$/
      const sshPattern = /^git@github\.com:([^/]+)\/([^/.]+?)(?:\.git)?$/

      let repoName: string | undefined

      // Try HTTPS pattern first
      let match = remoteUrl.match(httpsPattern)
      if (match) {
        repoName = match[1]
      } else {
        // Try SSH pattern
        match = remoteUrl.match(sshPattern)
        if (match) {
          repoName = match[2]
        }
      }

      if (!repoName) {
        toast.error('Please enter a valid GitHub repository URL (HTTPS or SSH)')
        return
      }

      repositoryData = {
        name: repoName, // Auto-extracted name
        remoteUrl: remoteUrl,
        sourceType: 'github',
        useGitWorktrees: false, // Default to false for GitHub repos
        codeExecution: {
          type: 'local',
        },
      }
    } else {
      // For local repos, use all form fields
      repositoryData = {
        name: formData.name.trim(),
        description: formData.description.trim() || undefined,
        path: formData.path.trim(),
        sourceType: 'local',
        useGitWorktrees: formData.useGitWorktrees,
        codeExecution: {
          type: 'local',
        },
      }
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
            onValueChange={(value) => value && setSourceType(value as 'local' | 'github')}
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
      ) : (
        <ConfigurationGroup title="GitHub Repository">
          <ConfigurationItem
            label="Repository URL"
            description="Enter the GitHub repository URL to clone"
          >
            <ConfigurationInput
              value={formData.remoteUrl}
              onChange={(e) => handleGitHubUrlChange(e.target.value)}
              placeholder="https://github.com/owner/repository or git@github.com:owner/repository.git"
              className={errors.remoteUrl ? 'border-destructive' : ''}
              autoFocus
            />
            {errors.remoteUrl && (
              <p className="text-sm text-destructive mt-1">{errors.remoteUrl}</p>
            )}
          </ConfigurationItem>
        </ConfigurationGroup>
      )}

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
