import type { RemoteRepository, RepositoryConfig } from '@moatless/types'
import { ExternalLink, Github } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateRemoteRepositoryFromLocal } from '@/features/repositories/hooks/use-repositories'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { CreateRepoDialog } from './create-repo-dialog'

interface CreateRepoButtonProps {
  repositoryId: string
  taskName: string
  repository: RepositoryConfig | undefined
}

export function CreateRepoButton({ repositoryId, taskName, repository }: CreateRepoButtonProps) {
  const [isDialogOpen, setIsDialogOpen] = useState(false)
  const createRepoMutation = useCreateRemoteRepositoryFromLocal()

  // Don't show if repository is not loaded or not a local repository
  if (!repository || repository.kind !== 'local') {
    // Check if it's a remote repository (already pushed to GitHub)
    if (repository?.kind === 'remote') {
      const remoteRepo = repository as RemoteRepository
      const githubUrl = remoteRepo.remote.url

      return (
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="outline" size="sm" className="h-8 gap-2">
              <Github className="h-4 w-4" />
              On GitHub
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-64 p-3" align="end">
            <div className="flex flex-col gap-2">
              <a
                href={githubUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center gap-2 text-sm text-primary hover:underline"
              >
                <ExternalLink className="h-4 w-4" />
                View on GitHub
              </a>
            </div>
          </PopoverContent>
        </Popover>
      )
    }

    return null
  }

  const handleSubmit = async (repositoryName: string, visibility: 'private' | 'public') => {
    try {
      const result = await createRepoMutation.mutateAsync({
        repoId: repositoryId,
        request: {
          repositoryName,
          visibility,
        },
      })

      setIsDialogOpen(false)

      // Extract GitHub URL from result
      if (result.kind === 'remote') {
        const remoteRepo = result as RemoteRepository
        toast.success('Repository created on GitHub', {
          description: (
            <a
              href={remoteRepo.remote.url}
              target="_blank"
              rel="noopener noreferrer"
              className="underline"
            >
              View {repositoryName} on GitHub
            </a>
          ),
        })
      } else {
        toast.success('Repository created on GitHub')
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Failed to create repository'
      toast.error('Failed to create GitHub repository', {
        description: errorMessage,
      })
    }
  }

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => setIsDialogOpen(true)}
        className="h-8 gap-2"
      >
        <Github className="h-4 w-4" />
        Create Repo
      </Button>

      <CreateRepoDialog
        isOpen={isDialogOpen}
        onOpenChange={setIsDialogOpen}
        defaultName={taskName}
        onSubmit={handleSubmit}
        isPending={createRepoMutation.isPending}
      />
    </>
  )
}
