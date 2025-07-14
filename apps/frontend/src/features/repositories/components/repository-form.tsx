import { RepositoryConfigSchema } from '@moatless/types/schemas'
import type {
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
} from '@moatless/types/schemas/repository.schemas'
import type {
  RepositoryResponse,
} from '@moatless/types/types/repository'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useId, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'
import { useZodForm } from '@/lib/hooks/use-zod-form'

import { LocalRepositoryConfig } from './local-repository-config'

interface RepositoryFormProps {
  repository?: RepositoryResponse
  onSubmit: (data: CreateRepositoryRequest | UpdateRepositoryRequest) => void
  isSubmitting?: boolean
}

type FormData = {
  name: string
  description: string
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
    ...getInitialRepositoryData(repository),
  }))


  // Use schema validation
  const { errors, validate, clearErrors } = useZodForm({ schema: RepositoryConfigSchema })

  useEffect(() => {
    if (repository) {
      setFormData({
        name: repository.name,
        description: repository.description || '',
        ...getInitialRepositoryData(repository),
      })
    }
  }, [repository, getInitialRepositoryData])

  // Clear errors when form data changes
  useEffect(() => {
    clearErrors()
  }, [clearErrors])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    const repositoryData: CreateRepositoryRequest | UpdateRepositoryRequest = {
      name: formData.name.trim(),
      description: formData.description.trim() || undefined,
      path: formData.path.trim(),
      useGitWorktrees: formData.useGitWorktrees,
      codeExecution: {
        type: 'local',
      },
    }

    // Validate using schema
    const result = validate(repositoryData)

    if (!result.success) {
      // Show first error as a toast
      const firstError = result.error.errors[0]
      if (firstError) {
        toast.error(firstError.message)
      }
      return
    }

    onSubmit(result.data)
  }


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
                  className={errors.name ? 'border-destructive' : ''}
                />
                {errors.name && <p className="text-sm text-destructive mt-1">{errors.name}</p>}
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
              pathError={errors.path}
            />

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
          <Button type="submit" disabled={isSubmitting}>
            {isSubmitting ? 'Saving...' : isEdit ? 'Update Repository' : 'Create Repository'}
          </Button>
        </div>
      </form>
    </div>
  )
}
