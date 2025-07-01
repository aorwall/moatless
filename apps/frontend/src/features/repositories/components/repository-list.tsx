import type { RepositoryResponse } from '@moatless/shared/types/repository'
import { Link } from '@tanstack/react-router'
import { Plus, Trash2 } from 'lucide-react'
import { useDeleteRepository } from '@/features/repositories/hooks/use-repositories'
import { Button } from '@/lib/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface RepositoryListProps {
  repositories?: Array<RepositoryResponse>
  isLoading: boolean
}

export function RepositoryList({ repositories, isLoading }: RepositoryListProps) {
  const deleteRepository = useDeleteRepository()

  const handleDelete = (repoId: string, repoName: string, event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    if (confirm(`Are you sure you want to delete the repository "${repoName}"?`)) {
      deleteRepository.mutate(repoId)
    }
  }

  if (isLoading) {
    return <div className="text-center py-8 text-muted-foreground">Loading repositories...</div>
  }

  if (!repositories || repositories.length === 0) {
    return (
      <div className="text-center py-8 space-y-4">
        <div className="text-muted-foreground">
          <p>No repositories configured yet.</p>
          <p className="text-sm mt-2">Create your first repository to get started.</p>
        </div>
        <Button asChild>
          <Link to="/repositories/new">
            <Plus className="h-4 w-4 mr-2" />
            New Repository
          </Link>
        </Button>
      </div>
    )
  }

  return (
    <TooltipProvider>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-2xl font-bold">Repositories</h2>
            <p className="text-muted-foreground">Manage your agent execution repositories</p>
          </div>
          <Button asChild>
            <Link to="/repositories/new">
              <Plus className="h-4 w-4 mr-2" />
              New Repository
            </Link>
          </Button>
        </div>
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[30%]">Name</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Path</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {repositories.map((repo) => (
                <TableRow key={repo.id} className="cursor-pointer">
                  <TableCell className="font-medium">
                    <Link
                      to="/repositories/$repositoryId"
                      params={{ repositoryId: repo.id }}
                      className="block hover:text-primary"
                    >
                      {repo.name}
                    </Link>
                  </TableCell>
                  <TableCell className="text-muted-foreground">local</TableCell>
                  <TableCell className="text-muted-foreground max-w-0">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="truncate block">{repo.path}</span>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p className="text-xs">{repo.path}</p>
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-2">
                      <Button variant="ghost" size="sm" asChild>
                        <Link to="/repositories/$repositoryId" params={{ repositoryId: repo.id }}>
                          Edit
                        </Link>
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={(e) => handleDelete(repo.id, repo.name, e)}
                        disabled={deleteRepository.isPending}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>
    </TooltipProvider>
  )
}
