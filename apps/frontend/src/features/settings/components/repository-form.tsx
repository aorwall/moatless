import { CreateRepositorySchema, UpdateRepositorySchema } from '@moatless/shared/schemas/repository.schemas'
import type {
  CodeExecution,
  CreateRepositoryRequest,
  Repository,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/shared/types/repository'

type LocalRepository = Extract<Repository, { type: 'local' }>
type GithubRepository = Extract<Repository, { type: 'github' }>

import { useId, useState } from 'react'
import { z } from 'zod'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Form, FormDescription, FormField, FormLabel, FormMessage } from '@/lib/components/ui/form'
import { Input } from '@/lib/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Switch } from '@/lib/components/ui/switch'
import { Textarea } from '@/lib/components/ui/textarea'

interface RepositoryFormProps {
  onSubmit: (data: CreateRepositoryRequest | UpdateRepositoryRequest) => void
  onCancel: () => void
  isSubmitting?: boolean
  initialData?: RepositoryResponse
  mode?: 'create' | 'edit'
}

export function RepositoryForm({
  onSubmit,
  onCancel,
  isSubmitting = false,
  initialData,
  mode = 'create',
}: RepositoryFormProps) {
  // Generate unique IDs for form fields
  const nameId = useId()
  const descriptionId = useId()
  const repositoryTypeId = useId()
  const localPathId = useId()
  const gitWorktreesId = useId()
  const githubRepoId = useId()
  const githubBranchId = useId()
  const githubCommitId = useId()
  const githubTokenId = useId()
  const codeExecutionTypeId = useId()
  const dockerImageId = useId()

  const getDefaultFormData = () => ({
    name: '',
    description: '',
    repositoryType: 'local' as 'local' | 'github',
    codeExecutionType: 'local' as 'local' | 'docker',
    // Repository fields
    localPath: '',
    useGitWorktrees: false,
    githubRepoId: '',
    githubBranch: 'main',
    githubCommit: '',
    githubToken: '',
    // Execution fields
    localSetupCommands: '',
    dockerImage: '',
    dockerEnvVars: {} as Record<string, string>,
    dockerSetupCommands: '',
    dockerVolumes: [] as Array<string>,
    dockerPorts: {} as Record<string, number>,
    // Variables
    variables: {} as Record<string, string>,
  })

  const getRepositoryFields = (repo: Repository) => {
    if (repo.type === 'local') {
      return {
        localPath: repo.path,
        useGitWorktrees: repo.useGitWorktrees,
        githubRepoId: '',
        githubBranch: 'main',
        githubCommit: '',
        githubToken: '',
      }
    }

    return {
      localPath: '',
      useGitWorktrees: false,
      githubRepoId: repo.repoId,
      githubBranch: repo.branch || 'main',
      githubCommit: repo.commit || '',
      githubToken: repo.token || '',
    }
  }

  const getExecutionFields = (exec: CodeExecution) => {
    if (exec.type === 'local') {
      return {
        localSetupCommands: exec.setupCommands || '',
        dockerImage: '',
        dockerEnvVars: {} as Record<string, string>,
        dockerSetupCommands: '',
        dockerVolumes: [] as Array<string>,
        dockerPorts: {} as Record<string, number>,
      }
    }

    return {
      localSetupCommands: '',
      dockerImage: exec.image || '',
      dockerEnvVars: exec.envVars || {},
      dockerSetupCommands: exec.setupCommands || '',
      dockerVolumes: exec.volumes || [],
      dockerPorts: exec.ports || {},
    }
  }

  const getInitialFormData = () => {
    if (!initialData) {
      return getDefaultFormData()
    }

    const repo = initialData.repository
    const exec = initialData.codeExecution

    return {
      name: initialData.name,
      description: initialData.description || '',
      repositoryType: repo.type,
      codeExecutionType: exec.type,
      ...getRepositoryFields(repo),
      ...getExecutionFields(exec),
      variables: initialData.variables || {},
    }
  }

  const [formData, setFormData] = useState(getInitialFormData())

  const [errors, setErrors] = useState<Record<string, string>>({})

  const validateForm = (): boolean => {
    try {
      if (mode === 'create') {
        const request: CreateRepositoryRequest = {
          name: formData.name,
          description: formData.description || undefined,
          repository: createRepository(),
          codeExecution: createCodeExecution(),
          variables: Object.keys(formData.variables).length > 0 ? formData.variables : undefined,
        }
        CreateRepositorySchema.parse(request)
      } else {
        const request: UpdateRepositoryRequest = {
          name: formData.name,
          description: formData.description || undefined,
          repository: createRepository(),
          codeExecution: createCodeExecution(),
          variables: Object.keys(formData.variables).length > 0 ? formData.variables : undefined,
        }
        UpdateRepositorySchema.parse(request)
      }
      setErrors({})
      return true
    } catch (error) {
      if (error instanceof z.ZodError) {
        const newErrors: Record<string, string> = {}
        error.errors.forEach((err) => {
          const path = err.path.join('.')
          if (path === 'name') {
            newErrors.name = err.message
          } else if (path === 'repository.path') {
            newErrors.localPath = err.message
          } else if (path === 'repository.repoId') {
            newErrors.githubRepoId = err.message
          } else if (path === 'codeExecution.image') {
            newErrors.dockerImage = err.message
          }
        })
        setErrors(newErrors)
        return false
      }
      throw error
    }
  }

  const createRepository = (): Repository => {
    if (formData.repositoryType === 'local') {
      return {
        type: 'local',
        path: formData.localPath,
        useGitWorktrees: formData.useGitWorktrees || undefined,
      } as LocalRepository
    }

    return {
      type: 'github',
      repoId: formData.githubRepoId,
      branch: formData.githubBranch || 'main',
      commit: formData.githubCommit || undefined,
      token: formData.githubToken || undefined,
    } as GithubRepository
  }

  const createCodeExecution = (): CodeExecution => {
    if (formData.codeExecutionType === 'local') {
      return {
        type: 'local',
        setupCommands: formData.localSetupCommands || undefined,
      }
    }

    return {
      type: 'docker',
      image: formData.dockerImage,
      envVars: formData.dockerEnvVars,
      setupCommands: formData.dockerSetupCommands || undefined,
      volumes: formData.dockerVolumes.length > 0 ? formData.dockerVolumes : undefined,
      ports: Object.keys(formData.dockerPorts).length > 0 ? formData.dockerPorts : undefined,
    }
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (!validateForm()) {
      return
    }

    const request =
      mode === 'create'
        ? ({
            name: formData.name,
            description: formData.description || undefined,
            repository: createRepository(),
            codeExecution: createCodeExecution(),
            variables: Object.keys(formData.variables).length > 0 ? formData.variables : undefined,
          } as CreateRepositoryRequest)
        : ({
            name: formData.name,
            description: formData.description || undefined,
            repository: createRepository(),
            codeExecution: createCodeExecution(),
            variables: Object.keys(formData.variables).length > 0 ? formData.variables : undefined,
          } as UpdateRepositoryRequest)

    onSubmit(request)
  }

  const updateFormData = <K extends keyof typeof formData>(
    field: K,
    value: (typeof formData)[K],
  ) => {
    setFormData((prev) => ({ ...prev, [field]: value }))
    // Clear error when user starts typing
    if (errors[field]) {
      setErrors((prev) => ({ ...prev, [field]: '' }))
    }
  }

  return (
    <Card className="w-full max-w-4xl">
      <CardHeader>
        <CardTitle>{mode === 'create' ? 'Create New Repository' : 'Edit Repository'}</CardTitle>
        <CardDescription>
          {mode === 'create'
            ? 'Configure a new repository with repository and execution settings'
            : 'Update the repository configuration'}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Form onSubmit={handleSubmit} className="space-y-6">
          {/* Basic Information */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium">Basic Information</h3>

            <FormField>
              <FormLabel htmlFor={nameId}>Name *</FormLabel>
              <Input
                id={nameId}
                value={formData.name}
                onChange={(e) => updateFormData('name', e.target.value)}
                placeholder="Repository name"
              />
              {errors.name && <FormMessage>{errors.name}</FormMessage>}
            </FormField>

            <FormField>
              <FormLabel htmlFor={descriptionId}>Description</FormLabel>
              <Textarea
                id={descriptionId}
                value={formData.description}
                onChange={(e) => updateFormData('description', e.target.value)}
                placeholder="Repository description"
                rows={3}
              />
            </FormField>
          </div>

          {/* Repository Configuration */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium">Repository</h3>

            <FormField>
              <FormLabel htmlFor={repositoryTypeId}>Repository Type</FormLabel>
              <Select
                value={formData.repositoryType}
                onValueChange={(value) =>
                  updateFormData('repositoryType', value as 'local' | 'github')
                }
              >
                <SelectTrigger id={repositoryTypeId} aria-label="Repository type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="github">GitHub</SelectItem>
                </SelectContent>
              </Select>
            </FormField>

            {formData.repositoryType === 'local' && (
              <div className="space-y-4">
                <FormField>
                  <FormLabel htmlFor={localPathId}>Local Path *</FormLabel>
                  <Input
                    id={localPathId}
                    value={formData.localPath}
                    onChange={(e) => updateFormData('localPath', e.target.value)}
                    placeholder="/path/to/your/project"
                  />
                  <FormDescription>Absolute path to the local repository</FormDescription>
                  {errors.localPath && <FormMessage>{errors.localPath}</FormMessage>}
                </FormField>

                <FormField>
                  <div className="flex items-center space-x-2">
                    <Switch
                      id={gitWorktreesId}
                      checked={formData.useGitWorktrees}
                      onCheckedChange={(checked) => updateFormData('useGitWorktrees', checked)}
                    />
                    <FormLabel htmlFor={gitWorktreesId} className="cursor-pointer">
                      Use Git Worktrees
                    </FormLabel>
                  </div>
                  <FormDescription>
                    Enable Git worktrees for better branch management
                  </FormDescription>
                </FormField>
              </div>
            )}

            {formData.repositoryType === 'github' && (
              <div className="space-y-4">
                <FormField>
                  <FormLabel htmlFor={githubRepoId}>Repository ID *</FormLabel>
                  <Input
                    id={githubRepoId}
                    value={formData.githubRepoId}
                    onChange={(e) => updateFormData('githubRepoId', e.target.value)}
                    placeholder="owner/repository"
                  />
                  <FormDescription>Format: owner/repository</FormDescription>
                  {errors.githubRepoId && <FormMessage>{errors.githubRepoId}</FormMessage>}
                </FormField>

                <div className="grid grid-cols-2 gap-4">
                  <FormField>
                    <FormLabel htmlFor={githubBranchId}>Branch</FormLabel>
                    <Input
                      id={githubBranchId}
                      value={formData.githubBranch}
                      onChange={(e) => updateFormData('githubBranch', e.target.value)}
                      placeholder="main"
                    />
                  </FormField>

                  <FormField>
                    <FormLabel htmlFor={githubCommitId}>Commit (optional)</FormLabel>
                    <Input
                      id={githubCommitId}
                      value={formData.githubCommit}
                      onChange={(e) => updateFormData('githubCommit', e.target.value)}
                      placeholder="commit hash"
                    />
                  </FormField>
                </div>

                <FormField>
                  <FormLabel htmlFor={githubTokenId}>Access Token (for private repos)</FormLabel>
                  <Input
                    id={githubTokenId}
                    type="password"
                    value={formData.githubToken}
                    onChange={(e) => updateFormData('githubToken', e.target.value)}
                    placeholder="github_pat_..."
                  />
                </FormField>
              </div>
            )}
          </div>

          {/* Code Execution Configuration */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium">Code Execution</h3>

            <FormField>
              <FormLabel htmlFor="code-execution-type">Execution Type</FormLabel>
              <Select
                value={formData.codeExecutionType}
                onValueChange={(value) =>
                  updateFormData('codeExecutionType', value as 'local' | 'docker')
                }
              >
                <SelectTrigger id={codeExecutionTypeId} aria-label="Code execution type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">Local</SelectItem>
                  <SelectItem value="docker">Docker</SelectItem>
                </SelectContent>
              </Select>
            </FormField>

            {formData.codeExecutionType === 'local' && (
              <div className="space-y-4">
                <FormField>
                  <FormLabel>Setup Commands</FormLabel>
                  <Textarea
                    value={formData.localSetupCommands}
                    onChange={(e) => updateFormData('localSetupCommands', e.target.value)}
                    placeholder="pip install -r requirements.txt"
                    rows={3}
                  />
                  <FormDescription>
                    Commands to run when setting up the repository (one per line)
                  </FormDescription>
                </FormField>
              </div>
            )}

            {formData.codeExecutionType === 'docker' && (
              <div className="space-y-4">
                <FormField>
                  <FormLabel htmlFor={dockerImageId}>Docker Image *</FormLabel>
                  <Input
                    id={dockerImageId}
                    value={formData.dockerImage}
                    onChange={(e) => updateFormData('dockerImage', e.target.value)}
                    placeholder="python:3.11"
                  />
                  {errors.dockerImage && <FormMessage>{errors.dockerImage}</FormMessage>}
                </FormField>

                <FormField>
                  <FormLabel>Setup Commands</FormLabel>
                  <Textarea
                    value={formData.dockerSetupCommands}
                    onChange={(e) => updateFormData('dockerSetupCommands', e.target.value)}
                    placeholder="apt-get update"
                    rows={3}
                  />
                  <FormDescription>
                    Commands to run when setting up the container (one per line)
                  </FormDescription>
                </FormField>
              </div>
            )}
          </div>

          {/* Form Actions */}
          <div className="flex justify-end space-x-4 pt-6 border-t">
            <Button type="button" variant="outline" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting
                ? mode === 'create'
                  ? 'Creating...'
                  : 'Updating...'
                : mode === 'create'
                  ? 'Create Repository'
                  : 'Update Repository'}
            </Button>
          </div>
        </Form>
      </CardContent>
    </Card>
  )
}
