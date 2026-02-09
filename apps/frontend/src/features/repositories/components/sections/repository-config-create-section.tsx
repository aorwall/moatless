import type { CreateRepositoryRequest } from '@moatless/api-client'
import type React from 'react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateRepository } from '@/features/repositories/hooks/use-repositories'

import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
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

  const [inputValue, setInputValue] = useState('')
  const [inputError, setInputError] = useState<string | null>(null)

  // Helper: Detect provider from Git URL
  const detectProvider = (url: string): 'github' | 'gitlab' => {
    if (url.includes('github.com')) {
      return 'github'
    }
    if (url.includes('gitlab.com')) {
      return 'gitlab'
    }
    throw new Error('Only GitHub and GitLab repositories are supported')
  }

  // Helper: Detect if input is a valid remote Git URL (GitHub or GitLab)
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

  const handleSubmit = () => {
    const trimmedInput = inputValue.trim()

    if (!trimmedInput) {
      toast.error('Please enter a Git URL')
      return
    }

    if (!isRemoteGitUrl(trimmedInput)) {
      toast.error(
        'Please enter a valid Git HTTPS URL (e.g., https://github.com/owner/repo or https://gitlab.com/group/project)',
      )
      return
    }

    // Create remote repository (GitHub or GitLab)
    const repoName = extractRepoNameFromUrl(trimmedInput)

    if (!repoName) {
      toast.error(
        'Please enter a valid Git HTTPS URL (e.g., https://github.com/owner/repo or https://gitlab.com/group/project)',
      )
      return
    }

    const provider = detectProvider(trimmedInput)

    const repositoryData: CreateRepositoryRequest = {
      kind: 'remote',
      name: repoName,
      remoteUrl: trimmedInput,
      provider: provider,
    }

    createRepository.mutate(repositoryData, {
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
    setInputError(null)
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Add Repository">
        <ConfigurationItem
          label="Repository URL"
          description="Enter a Git HTTPS URL (GitHub/GitLab)"
        >
          <ConfigurationInput
            value={inputValue}
            onChange={handleInputChange}
            placeholder="https://github.com/owner/repo or https://gitlab.com/group/project"
            className={inputError ? 'border-destructive' : ''}
            autoFocus
          />
          {inputError && <p className="text-sm text-destructive mt-1">{inputError}</p>}
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
