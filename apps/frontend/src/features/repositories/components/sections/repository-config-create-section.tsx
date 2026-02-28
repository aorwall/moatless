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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { getErrorMessage } from '@/lib/utils/error'

type GitProvider = 'github' | 'gitlab' | 'gitness'

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
  const [provider, setProvider] = useState<GitProvider>('github')

  // Helper: Auto-detect provider from Git URL (convenience — user can override)
  const autoDetectProvider = (url: string): GitProvider | null => {
    if (url.includes('github.com')) {
      return 'github'
    }
    if (url.includes('gitlab.com')) {
      return 'gitlab'
    }
    return null
  }

  // Helper: Validate URL based on selected provider
  const isValidGitUrl = (input: string, selectedProvider: GitProvider): boolean => {
    const trimmed = input.trim()
    if (!trimmed.startsWith('https://')) {
      return false
    }
    if (selectedProvider === 'github') {
      return trimmed.startsWith('https://github.com/')
    }
    if (selectedProvider === 'gitlab') {
      return trimmed.startsWith('https://gitlab.com/')
    }
    // Gitness: any HTTPS URL is valid (self-hosted)
    return true
  }

  // Helper: Extract repository name from Git HTTPS URL
  const extractRepoNameFromUrl = (url: string): string | null => {
    const cleaned = url
      .trim()
      .replace(/\/$/, '')
      .replace(/\.git$/, '')
    const parts = cleaned.split('/')
    // Return the last path segment as repo name
    const lastPart = parts[parts.length - 1]
    return lastPart && lastPart.length > 0 ? lastPart : null
  }

  const handleSubmit = () => {
    const trimmedInput = inputValue.trim()

    if (!trimmedInput) {
      toast.error('Please enter a Git URL')
      return
    }

    if (!isValidGitUrl(trimmedInput, provider)) {
      if (provider === 'gitness') {
        toast.error('Please enter a valid HTTPS URL for your Gitness repository')
      } else {
        toast.error(`Please enter a valid ${provider === 'github' ? 'GitHub' : 'GitLab'} HTTPS URL`)
      }
      return
    }

    const repoName = extractRepoNameFromUrl(trimmedInput)

    if (!repoName) {
      toast.error('Could not extract repository name from URL')
      return
    }

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
    const value = e.target.value
    setInputValue(value)
    setInputError(null)

    // Auto-detect provider from URL as convenience
    const detected = autoDetectProvider(value)
    if (detected) {
      setProvider(detected)
    }
  }

  const handleProviderChange = (value: string) => {
    setProvider(value as GitProvider)
    setInputError(null)
  }

  const getPlaceholder = (): string => {
    switch (provider) {
      case 'github':
        return 'https://github.com/owner/repo'
      case 'gitlab':
        return 'https://gitlab.com/group/project'
      case 'gitness':
        return 'https://gitness.example.com/org/repo'
    }
  }

  return (
    <ConfigurationSection>
      <ConfigurationGroup title="Add Repository">
        <ConfigurationItem label="Git Provider" description="Select the hosting provider">
          <Select value={provider} onValueChange={handleProviderChange}>
            <SelectTrigger className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="github">GitHub</SelectItem>
              <SelectItem value="gitlab">GitLab</SelectItem>
              <SelectItem value="gitness">Gitness</SelectItem>
            </SelectContent>
          </Select>
        </ConfigurationItem>

        <ConfigurationItem label="Repository URL" description="Enter the Git HTTPS URL">
          <ConfigurationInput
            value={inputValue}
            onChange={handleInputChange}
            placeholder={getPlaceholder()}
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
