import type { UpdateEnvironmentRequest } from '@shared/types/environment'
import { createFileRoute } from '@tanstack/react-router'
import { toast } from 'sonner'
import { EnvironmentForm, useEnvironment, useUpdateEnvironment } from '@/features/environments'
import { CodeExecutionConfig } from '@/features/environments/components/code-execution-config'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/environments/$environmentId')({
  component: EnvironmentEdit,
})

function EnvironmentEdit() {
  const { environmentId } = Route.useParams()

  const { data: environment, isLoading, error } = useEnvironment(environmentId)
  const updateEnvironment = useUpdateEnvironment()

  const handleSubmit = (data: UpdateEnvironmentRequest) => {
    updateEnvironment.mutate(
      { envId: environmentId, request: data },
      {
        onSuccess: () => {
          toast.success('Environment updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update environment', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleCodeExecutionUpdate = (codeExecution: UpdateEnvironmentRequest['code_execution']) => {
    if (!codeExecution) {
      return
    }

    updateEnvironment.mutate(
      { envId: environmentId, request: { code_execution: codeExecution } },
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

  if (error || !environment) {
    return (
      <div className="container mx-auto py-6">
        <div className="max-w-2xl mx-auto text-center">
          <p className="text-destructive">{error ? error.message : 'Environment not found'}</p>
        </div>
      </div>
    )
  }

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-2xl mx-auto space-y-6">
        <EnvironmentForm
          environment={environment}
          onSubmit={handleSubmit}
          isSubmitting={updateEnvironment.isPending}
        />
        <CodeExecutionConfig
          codeExecution={environment.code_execution}
          onUpdate={handleCodeExecutionUpdate}
          isSubmitting={updateEnvironment.isPending}
          environmentId={environmentId}
        />
      </div>
    </div>
  )
}
