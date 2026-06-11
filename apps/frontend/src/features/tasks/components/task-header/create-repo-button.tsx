import type { RepositoryResponse } from '@moatless/api-client'
import { ExternalLink } from 'lucide-react'
import { Github } from '@/lib/components/ui/brand-icons'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

interface CreateRepoButtonProps {
  repositoryId: string
  taskName: string
  repository: RepositoryResponse | undefined
}

export function CreateRepoButton({ repository }: CreateRepoButtonProps) {
  // Don't show if repository is not loaded
  if (!repository) {
    return null
  }

  // Show GitHub link for remote repositories
  if (repository.kind === 'remote') {
    const githubUrl = repository.remoteUrl

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
              href={githubUrl ?? ''}
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

  // Templates don't show a button
  return null
}
