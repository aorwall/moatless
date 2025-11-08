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

  // Helper: Extract repository name from GitHub HTTPS URL
  const extractRepoNameFromUrl = (url: string): string | null => {
    const regex = /^https:\/\/github\.com\/[^/]+\/([^/.]+)(?:\.git)?$/
    const match = url.match(regex)
    return match?.[1] || null
  }

  // Helper: Extract name from local path
  const extractNameFromPath = (path: string): string => {
    const trimmedPath = path.trim().replace(/\/+$/, '') // Remove trailing slashes
    const segments = trimmedPath.split('/')
    return segments[segments.length - 1] || 'repository'
  }

  // Helper: Detect if input is GitHub URL or local path
  const isGitHubUrl = (input: string): boolean => {
    return input.trim().startsWith('https://github.com/')
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
      toast.error('Please enter a repository path or GitHub URL')
      return
    }

    let repositoryData: CreateRepositoryRequest

    if (isGitHubUrl(trimmedInput)) {
      // Create remote repository
      const repoName = extractRepoNameFromUrl(trimmedInput)

      if (!repoName) {
        toast.error('Please enter a valid GitHub HTTPS URL (e.g., https://github.com/owner/repo)')
        return
      }

      repositoryData = {
        kind: 'remote',
        name: repoName,
        remoteUrl: trimmedInput,
        provider: 'github',
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
          description="Enter a local file path or GitHub HTTPS URL - we'll automatically detect the type"
        >
          <ConfigurationInput
            value={inputValue}
            onChange={handleInputChange}
            placeholder="/path/to/repo or https://github.com/owner/repo"
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
