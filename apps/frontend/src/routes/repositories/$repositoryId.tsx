import type { PreviewConfig, UpdateRepositoryRequest } from '@moatless/shared/types/repository'
import { createFileRoute } from '@tanstack/react-router'
import { toast } from 'sonner'
import {
  PreviewConfigForm,
  RepositoryForm,
  useDetectFramework,
  useRepository,
  useUpdateRepository,
} from '@/features/repositories'
import { CodeExecutionConfig } from '@/features/repositories/components/code-execution-config'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/repositories/$repositoryId')({
  component: RepositoryEdit,
})

function RepositoryEdit() {
  const { repositoryId } = Route.useParams()

  const { data: repository, isLoading, error } = useRepository(repositoryId)
  const updateRepository = useUpdateRepository()
  const detectFramework = useDetectFramework(repositoryId)

  const handleSubmit = (data: UpdateRepositoryRequest) => {
    updateRepository.mutate(
      { repoId: repositoryId, request: data },
      {
        onSuccess: () => {
          toast.success('Repository updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update repository', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleCodeExecutionUpdate = (codeExecution: UpdateRepositoryRequest['codeExecution']) => {
    if (!codeExecution) {
      return
    }

    updateRepository.mutate(
      { repoId: repositoryId, request: { codeExecution: codeExecution } },
      {
        onSuccess: () => {
          toast.success('Code execution settings updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update code execution settings', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handlePreviewConfigUpdate = (previewConfig: UpdateRepositoryRequest['previewConfig']) => {
    updateRepository.mutate(
      { repoId: repositoryId, request: { previewConfig } },
      {
        onSuccess: () => {
          toast.success('Preview configuration updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update preview configuration', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleDetectFramework = async (): Promise<PreviewConfig | undefined> => {
    try {
      const result = await detectFramework.mutateAsync()
      toast.success('Framework detected successfully')
      return result as PreviewConfig
    } catch (error) {
      toast.error('Failed to detect framework', {
        description: getErrorMessage(error),
      })
      return undefined
    }
  }

  if (isLoading) {
    return (
      <div className="container mx-auto py-6">
        <div className="max-w-2xl mx-auto space-y-4">
          <Skeleton className="h-32 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      </div>
    )
  }

  if (error || !repository) {
    return (
      <div className="container mx-auto py-6">
        <div className="max-w-2xl mx-auto text-center">
          <p className="text-destructive">{error ? error.message : 'Repository not found'}</p>
        </div>
      </div>
    )
  }

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-2xl mx-auto space-y-6">
        <RepositoryForm
          repository={repository}
          onSubmit={handleSubmit}
          isSubmitting={updateRepository.isPending}
        />
        <CodeExecutionConfig
          codeExecution={repository.codeExecution}
          onUpdate={handleCodeExecutionUpdate}
          isSubmitting={updateRepository.isPending}
          repositoryId={repositoryId}
        />
        <PreviewConfigForm
          previewConfig={repository.previewConfig}
          onUpdate={handlePreviewConfigUpdate}
          isSubmitting={updateRepository.isPending}
          repositoryId={repositoryId}
          onDetect={handleDetectFramework}
        />
      </div>
    </div>
  )
}
