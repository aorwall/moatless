import type {
  CreateEnvironmentRequest,
  EnvironmentResponse,
  UpdateEnvironmentRequest,
} from '@shared/types/environmenttypes/environment'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

import { LocalRepositoryConfig } from './local-repository-config'

interface EnvironmentFormProps {
  environment?: EnvironmentResponse
  onSubmit: (data: CreateEnvironmentRequest | UpdateEnvironmentRequest) => void
  isSubmitting?: boolean
}

type FormData = {
  name: string
  type: 'local' | 'github'
  environment_variables: Record<string, string>
  isActive: boolean
  path: string
  useGitWorktrees: boolean
  repo_url: string
  branch: string
}

export function EnvironmentForm({ environment, onSubmit, isSubmitting }: EnvironmentFormProps) {
  const navigate = useNavigate()
  const isEdit = !!environment

  const getInitialRepositoryData = useCallback((env?: EnvironmentResponse) => {
    if (env?.repository.type === 'local') {
      return {
        path: env.repository.path,
        useGitWorktrees: env.repository.useGitWorktrees ?? false,
      }
    } else if (env?.repository.type === 'github') {
      return {
        repo_url: env.repository.repoId,
        branch: env.repository.branch || 'main',
      }
    }
    return {
      path: '',
      useGitWorktrees: false,
      repo_url: '',
      branch: 'main',
    }
  }, [])

  const [formData, setFormData] = useState<FormData>({
    name: environment?.name || '',
    type: environment?.repository.type || 'local',
    environment_variables: environment?.variables || {},
    isActive: environment?.is_active ?? true,
    ...getInitialRepositoryData(environment),
  } as FormData)

  // Update form data when environment prop changes
  useEffect(() => {
    if (environment) {
      const repoData = getInitialRepositoryData(environment)
      setFormData({
        name: environment.name,
        type: environment.repository.type,
        environment_variables: environment.variables || {},
        isActive: environment.is_active,
        path: 'path' in repoData ? (repoData.path as string) : '',
        useGitWorktrees:
          'useGitWorktrees' in repoData ? (repoData.useGitWorktrees as boolean) : false,
        repo_url: 'repo_url' in repoData ? (repoData.repo_url as string) : '',
        branch: 'branch' in repoData ? (repoData.branch as string) : 'main',
      })
    }
  }, [environment, getInitialRepositoryData]) // Re-run when environment changes

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (!formData.name.trim()) {
      toast.error('Please provide an environment name')
      return
    }

    const repository =
      formData.type === 'local'
        ? {
            type: 'local' as const,
            path: formData.path,
            useGitWorktrees: formData.useGitWorktrees,
          }
        : {
            type: 'github' as const,
            repoId: formData.repo_url || '',
            branch: formData.branch,
          }

    const codeExecution = {
      type: 'local' as const,
    }

    const submitData = isEdit
      ? {
          name: formData.name,
          repository,
          codeExecution,
          variables: formData.environment_variables,
          isActive: formData.isActive,
        }
      : {
          name: formData.name,
          repository,
          codeExecution,
          variables: formData.environment_variables,
        }

    onSubmit(submitData)
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{isEdit ? 'Environment Repository Settings' : 'Create Environment'}</CardTitle>
        <CardDescription>
          {isEdit
            ? 'Update repository configuration'
            : 'Configure a new environment with repository settings'}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-6">
          <div className="space-y-4">
            <div>
              <Label htmlFor="name">Environment Name</Label>
              <Input
                id="name"
                value={formData.name}
                onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                placeholder="Production Environment"
                required
              />
            </div>

            <div>
              <Label htmlFor="type">Environment Type</Label>
              <Select
                value={formData.type}
                onValueChange={(value) =>
                  setFormData({
                    ...formData,
                    type: value as 'local' | 'github',
                  })
                }
                disabled={isEdit}
              >
                <SelectTrigger id="type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="github">GitHub</SelectItem>
                </SelectContent>
              </Select>
              {isEdit && (
                <p className="text-sm text-muted-foreground mt-1">
                  Type cannot be changed after creation
                </p>
              )}
            </div>

            {formData.type === 'local' && (
              <LocalRepositoryConfig
                path={formData.path}
                useGitWorktrees={formData.useGitWorktrees}
                onPathChange={(path) => setFormData({ ...formData, path })}
                onUseGitWorktreesChange={(useGitWorktrees) =>
                  setFormData({ ...formData, useGitWorktrees })
                }
              />
            )}

            {formData.type === 'github' && (
              <>
                <div>
                  <Label htmlFor="repo_url">Repository (owner/repo)</Label>
                  <Input
                    id="repo_url"
                    value={formData.repo_url}
                    onChange={(e) => setFormData({ ...formData, repo_url: e.target.value })}
                    placeholder="example/repo"
                  />
                </div>

                <div>
                  <Label htmlFor="branch">Branch</Label>
                  <Input
                    id="branch"
                    value={formData.branch}
                    onChange={(e) => setFormData({ ...formData, branch: e.target.value })}
                    placeholder="main"
                  />
                </div>
              </>
            )}
          </div>

          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => navigate({ to: '/environments' })}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting ? 'Saving...' : isEdit ? 'Update' : 'Create'}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}
