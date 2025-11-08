import type {
  CreateLocalRepositoryRequest,
  CreateRemoteRepositoryRequest,
  CreateRepositoryRequest,
} from '@moatless/types'
import { CreateRepositorySchema } from '@moatless/types'
import { Github, HardDrive } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import type { z } from 'zod'
import { useCreateRepository } from '@/features/repositories/hooks/use-repositories'

type LocalFormData = {
  kind: 'local'
  name: string
  description: string
  path: string
  useGitWorktrees: boolean
  systemPrompt?: string
}

type RemoteFormData = {
  kind: 'remote'
  remoteUrl: string
  systemPrompt?: string
}

type RepositoryFormData = LocalFormData | RemoteFormData

type LocalField = keyof Omit<LocalFormData, 'kind'>
type RemoteField = keyof Omit<RemoteFormData, 'kind'>

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

  const [kind, setKind] = useState<'local' | 'remote'>('local')
  const [formData, setFormData] = useState<RepositoryFormData>({
    kind: 'local',
    name: '',
    description: '',
    path: '',
    useGitWorktrees: false,
  })

  // Helper: Extract repository name from GitHub HTTPS URL
  const extractRepoNameFromUrl = (url: string): string | null => {
    // Only support GitHub HTTPS URLs
    const regex = /^https:\/\/github\.com\/[^/]+\/([^/.]+)(?:\.git)?$/
    const match = url.match(regex)
    return match?.[1] || null
  }

  // Helper: Create repository data for remote source
  const createRemoteRepositoryData = (
    remoteUrl: string,
  ): CreateRemoteRepositoryRequest | { error: string } => {
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
      kind: 'remote',
      name: repoName,
      remoteUrl,
      provider: 'github',
      systemPrompt: formData.systemPrompt?.trim() || undefined,
      codeExecution: {},
    }
  }

  // Helper: Create repository data for local source
  const createLocalRepositoryData = (): CreateLocalRepositoryRequest => {
    if (formData.kind !== 'local') {
      throw new Error('Invalid form data kind')
    }

    return {
      kind: 'local',
      name: formData.name.trim(),
      description: formData.description.trim() || undefined,
      path: formData.path.trim(),
      useGitWorktrees: formData.useGitWorktrees,
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

    if (kind === 'remote') {
      if (formData.kind !== 'remote') {
        toast.error('Invalid form state')
        return
      }
      const result = createRemoteRepositoryData(formData.remoteUrl.trim())
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

  const handleLocalFieldChange = <Field extends LocalField>(
    field: Field,
    value: LocalFormData[Field],
  ) => {
    setFormData((prev) => {
      if (prev.kind !== 'local') {
        return prev
      }
      return { ...prev, [field]: value }
    })
    clearErrors()
  }

  const handleRemoteFieldChange = <Field extends RemoteField>(
    field: Field,
    value: RemoteFormData[Field],
  ) => {
    setFormData((prev) => {
      if (prev.kind !== 'remote') {
        return prev
      }
      return { ...prev, [field]: value }
    })
    clearErrors()
  }

  const handleKindChange = (value: string | null) => {
    if (value === 'local') {
      setKind('local')
      setFormData({
        kind: 'local',
        name: '',
        description: '',
        path: '',
        useGitWorktrees: false,
      })
    } else if (value === 'remote') {
      setKind('remote')
      setFormData({
        kind: 'remote',
        remoteUrl: '',
      })
    }
    clearErrors()
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Repository Source">
        <ConfigurationItem
          label="Repository Type"
          description="Choose between a local repository or a remote GitHub repository"
        >
          <ToggleGroup
            type="single"
            value={kind}
            onValueChange={handleKindChange}
            className="justify-start"
          >
            <ToggleGroupItem value="local" aria-label="Local repository">
              <HardDrive className="h-4 w-4 mr-2" />
              Local Repository
            </ToggleGroupItem>
            <ToggleGroupItem value="remote" aria-label="Remote GitHub repository">
              <Github className="h-4 w-4 mr-2" />
              Remote Repository
            </ToggleGroupItem>
          </ToggleGroup>
        </ConfigurationItem>
      </ConfigurationGroup>

      {kind === 'local' && formData.kind === 'local' ? (
        <ConfigurationGroup title="Repository Details">
          <ConfigurationItem label="Repository Name" description="Display name for your repository">
            <ConfigurationInput
              value={formData.name}
              onChange={(e) => handleLocalFieldChange('name', e.target.value)}
              placeholder="My Project Repository"
              className={(errors as { name?: string }).name ? 'border-destructive' : ''}
            />
            {(errors as { name?: string }).name && (
              <p className="text-sm text-destructive mt-1">{(errors as { name?: string }).name}</p>
            )}
          </ConfigurationItem>

          <ConfigurationItem
            label="Description"
            description="Optional description of this repository"
          >
            <ConfigurationTextarea
              value={formData.description}
              onChange={(e) => handleLocalFieldChange('description', e.target.value)}
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
              onChange={(e) => handleLocalFieldChange('path', e.target.value)}
              placeholder="/Users/username/projects/my-project"
              className={(errors as { path?: string }).path ? 'border-destructive' : ''}
            />
            {(errors as { path?: string }).path && (
              <p className="text-sm text-destructive mt-1">{(errors as { path?: string }).path}</p>
            )}
          </ConfigurationItem>

          <ConfigurationItem
            label="Git Worktrees"
            description="Use git worktrees for isolated environments (recommended for git repositories)"
          >
            <ConfigurationSwitch
              checked={formData.useGitWorktrees}
              onCheckedChange={(checked) => handleLocalFieldChange('useGitWorktrees', checked)}
            />
          </ConfigurationItem>
        </ConfigurationGroup>
      ) : kind === 'remote' && formData.kind === 'remote' ? (
        <ConfigurationGroup title="Remote Repository">
          <ConfigurationItem
            label="Repository URL"
            description="Enter the GitHub HTTPS repository URL (repository will be cloned for each task)"
          >
            <ConfigurationInput
              value={formData.remoteUrl}
              onChange={(e) => handleRemoteFieldChange('remoteUrl', e.target.value)}
              placeholder="https://github.com/owner/repo"
              className={(errors as { remoteUrl?: string }).remoteUrl ? 'border-destructive' : ''}
              autoFocus
            />
            {(errors as { remoteUrl?: string }).remoteUrl && (
              <p className="text-sm text-destructive mt-1">
                {(errors as { remoteUrl?: string }).remoteUrl}
              </p>
            )}
          </ConfigurationItem>
        </ConfigurationGroup>
      ) : null}

      <AgentConfigurationGroup
        formData={formData}
        onUpdateField={(field, value) => {
          if (field === 'systemPrompt') {
            if (kind === 'local') {
              handleLocalFieldChange('systemPrompt', value as string | undefined)
            } else if (kind === 'remote') {
              handleRemoteFieldChange('systemPrompt', value as string | undefined)
            }
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
