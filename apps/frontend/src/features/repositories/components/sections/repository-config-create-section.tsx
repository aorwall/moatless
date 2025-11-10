import type { CreateRepositoryRequest } from '@moatless/types'
import { CreateRepositorySchema } from '@moatless/types'
import { useState } from 'react'
import { toast } from 'sonner'
import type { z } from 'zod'
import { useCreateRepository } from '@/features/repositories/hooks/use-repositories'

import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'

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

  const [inputValue, setInputValue] = useState('')

  // Helper: Detect provider from Git URL
  const detectProvider = (url: string): 'github' | 'gitlab' | 'other' => {
    if (url.includes('github.com')) {
      return 'github'
    }
    if (url.includes('gitlab.com')) {
      return 'gitlab'
    }
    return 'other'
  }

  // Helper: Detect if input is a remote Git URL (GitHub or GitLab)
  const isRemoteGitUrl = (input: string): boolean => {
    const trimmed = input.trim()
    return trimmed.startsWith('https://github.com/') || trimmed.startsWith('https://gitlab.com/')
  }

  // Helper: Extract repository name from Git HTTPS URL (GitHub or GitLab)
  const extractRepoNameFromUrl = (url: string): string | null => {
    // Supports both GitHub (owner/repo) and GitLab (group/.../project)
    const regex = /^https:\/\/(github|gitlab)\.com\/(.+?)\/([^/.]+?)(?:\.git)?$/
    const match = url.match(regex)
    if (!match) {
      return null
    }
    // Return the last segment (repo/project name)
    return match[3] || null
  }

  // Helper: Extract name from local path
  const extractNameFromPath = (path: string): string => {
    const trimmedPath = path.trim().replace(/\/+$/, '') // Remove trailing slashes
    const segments = trimmedPath.split('/')
    return segments[segments.length - 1] || 'repository'
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
    const trimmedInput = inputValue.trim()

    if (!trimmedInput) {
      toast.error('Please enter a repository path or Git URL')
      return
    }

    let repositoryData: CreateRepositoryRequest

    if (isRemoteGitUrl(trimmedInput)) {
      // Create remote repository (GitHub or GitLab)
      const repoName = extractRepoNameFromUrl(trimmedInput)

      if (!repoName) {
        toast.error(
          'Please enter a valid Git HTTPS URL (e.g., https://github.com/owner/repo or https://gitlab.com/group/project)',
        )
        return
      }

      const provider = detectProvider(trimmedInput)

      repositoryData = {
        kind: 'remote',
        name: repoName,
        remoteUrl: trimmedInput,
        provider: provider,
        codeExecution: {},
      }
    } else {
      // Create local repository
      const repoName = extractNameFromPath(trimmedInput)

      repositoryData = {
        kind: 'local',
        name: repoName,
        path: trimmedInput,
        useGitWorktrees: false,
        codeExecution: {},
      }
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

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setInputValue(e.target.value)
    clearErrors()
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Add Repository">
        <ConfigurationItem
          label="Repository Path or URL"
          description="Enter a local file path or Git HTTPS URL (GitHub/GitLab) - we'll automatically detect the type"
        >
          <ConfigurationInput
            value={inputValue}
            onChange={handleInputChange}
            placeholder="/path/to/repo or https://github.com/owner/repo or https://gitlab.com/group/project"
            className={
              (errors as { path?: string; remoteUrl?: string }).path ||
              (errors as { path?: string; remoteUrl?: string }).remoteUrl
                ? 'border-destructive'
                : ''
            }
            autoFocus
          />
          {((errors as { path?: string }).path || (errors as { remoteUrl?: string }).remoteUrl) && (
            <p className="text-sm text-destructive mt-1">
              {(errors as { path?: string }).path || (errors as { remoteUrl?: string }).remoteUrl}
            </p>
          )}
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
