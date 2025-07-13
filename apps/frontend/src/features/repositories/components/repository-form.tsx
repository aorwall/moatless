import type {
  CreateRepositoryRequest,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/types/types/repository'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useId, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

import { LocalRepositoryConfig } from './local-repository-config'

interface RepositoryFormProps {
  repository?: RepositoryResponse
  onSubmit: (data: CreateRepositoryRequest | UpdateRepositoryRequest) => void
  isSubmitting?: boolean
}

type FormData = {
  name: string
  description: string
  variables: Record<string, string>
  path: string
  useGitWorktrees: boolean
}

export function RepositoryForm({ repository, onSubmit, isSubmitting }: RepositoryFormProps) {
  const navigate = useNavigate()
  const isEdit = !!repository

  const nameId = useId()
  const descriptionId = useId()

  const getInitialRepositoryData = useCallback((repo?: RepositoryResponse) => {
    if (repo) {
      return {
        path: repo.path,
        useGitWorktrees: repo.useGitWorktrees || false,
      }
    }
    return {
      path: '',
      useGitWorktrees: false,
    }
  }, [])

  const [formData, setFormData] = useState<FormData>(() => ({
    name: repository?.name || '',
    description: repository?.description || '',
    variables: repository?.variables || {},
    ...getInitialRepositoryData(repository),
  }))

  const [envVarKey, setEnvVarKey] = useState('')
  const [envVarValue, setEnvVarValue] = useState('')

  useEffect(() => {
    if (repository) {
      setFormData({
        name: repository.name,
        description: repository.description || '',
        variables: repository.variables || {},
        ...getInitialRepositoryData(repository),
      })
    }
  }, [repository, getInitialRepositoryData])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    const repositoryData: CreateRepositoryRequest | UpdateRepositoryRequest = {
      name: formData.name,
      description: formData.description || undefined,
      path: formData.path,
      useGitWorktrees: formData.useGitWorktrees,
      codeExecution: {
        type: 'local',
      },
      variables: Object.keys(formData.variables).length > 0 ? formData.variables : undefined,
    }

    onSubmit(repositoryData)
  }

  const handleAddEnvVar = () => {
    if (envVarKey && envVarValue) {
      setFormData({
        ...formData,
        variables: {
          ...formData.variables,
          [envVarKey]: envVarValue,
        },
      })
      setEnvVarKey('')
      setEnvVarValue('')
    }
  }

  const handleRemoveEnvVar = (key: string) => {
    const { [key]: _, ...rest } = formData.variables
    setFormData({
      ...formData,
      variables: rest,
    })
  }

  const isValid = formData.name.trim() && formData.path.trim()

  return (
    <div className="container mx-auto py-6 max-w-4xl">
      <form onSubmit={handleSubmit} className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle>{isEdit ? 'Edit Repository' : 'Create New Repository'}</CardTitle>
            <CardDescription>Configure a local repository for agent execution</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <Label htmlFor={nameId}>Repository Name</Label>
                <Input
                  id={nameId}
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  placeholder="My Project Repository"
                  required
                />
              </div>

              <div>
                <Label htmlFor={descriptionId}>Description</Label>
                <Textarea
                  id={descriptionId}
                  value={formData.description}
                  onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                  placeholder="Optional description of this repository"
                  rows={3}
                />
              </div>
            </div>

            <LocalRepositoryConfig
              path={formData.path}
              useGitWorktrees={formData.useGitWorktrees}
              onPathChange={(path) => setFormData({ ...formData, path })}
              onUseGitWorktreesChange={(useGitWorktrees) =>
                setFormData({ ...formData, useGitWorktrees })
              }
            />

            <div>
              <div className="flex items-center justify-between mb-2">
                <Label>Repository Variables</Label>
              </div>
              <div className="space-y-2">
                {Object.entries(formData.variables).map(([key, value]) => (
                  <div key={key} className="flex items-center gap-2">
                    <Input value={key} disabled className="flex-1" />
                    <Input value={value} disabled className="flex-1" />
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => handleRemoveEnvVar(key)}
                    >
                      Remove
                    </Button>
                  </div>
                ))}
                <div className="flex items-center gap-2">
                  <Input
                    placeholder="Variable name"
                    value={envVarKey}
                    onChange={(e) => setEnvVarKey(e.target.value)}
                    className="flex-1"
                  />
                  <Input
                    placeholder="Variable value"
                    value={envVarValue}
                    onChange={(e) => setEnvVarValue(e.target.value)}
                    className="flex-1"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleAddEnvVar}
                    disabled={!(envVarKey && envVarValue)}
                  >
                    Add
                  </Button>
                </div>
              </div>
              <p className="text-sm text-muted-foreground mt-1">
                Repository-specific variables that will be available during execution
              </p>
            </div>
          </CardContent>
        </Card>

        <div className="flex justify-end gap-4">
          <Button
            type="button"
            variant="outline"
            onClick={() => navigate({ to: '/repositories' })}
            disabled={isSubmitting}
          >
            Cancel
          </Button>
          <Button type="submit" disabled={isSubmitting || !isValid}>
            {isSubmitting ? 'Saving...' : isEdit ? 'Update Repository' : 'Create Repository'}
          </Button>
        </div>
      </form>
    </div>
  )
}
