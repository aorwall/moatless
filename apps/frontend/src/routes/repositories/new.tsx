import type {
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
} from '@moatless/types'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'
import { RepositoryForm, useCreateRepository } from '@/features/repositories'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/repositories/new')({
  component: RepositoryNew,
})

function RepositoryNew() {
  const navigate = useNavigate()
  const createRepository = useCreateRepository()

  const handleSubmit = (data: CreateRepositoryRequest | UpdateRepositoryRequest) => {
    createRepository.mutate(data as CreateRepositoryRequest, {
      onSuccess: (createdRepository) => {
        toast.success('Repository created successfully')
        // Navigate to the edit page to configure code execution
        navigate({
          to: '/repositories/$repositoryId',
          params: { repositoryId: createdRepository.id },
        })
      },
      onError: (error) => {
        toast.error('Failed to create repository', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-2xl mx-auto">
        <RepositoryForm onSubmit={handleSubmit} isSubmitting={createRepository.isPending} />
      </div>
    </div>
  )
}
