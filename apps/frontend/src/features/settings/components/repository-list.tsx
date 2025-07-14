import type { RepositoryResponse } from '@moatless/types/types/repository'
import { useState } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'

interface RepositoryListProps {
  repositories: Array<RepositoryResponse>
  onCreateNew: () => void
  onEdit: (repository: RepositoryResponse) => void
  onDelete: (repositoryId: string) => void
  isLoading?: boolean
}

export function RepositoryList({
  repositories,
  onCreateNew,
  onEdit,
  onDelete,
  isLoading = false,
}: RepositoryListProps) {
  const [deletingId, setDeletingId] = useState<string | null>(null)

  const handleDelete = async (repositoryId: string) => {
    if (confirm('Are you sure you want to delete this repository? This action cannot be undone.')) {
      setDeletingId(repositoryId)
      try {
        await onDelete(repositoryId)
      } finally {
        setDeletingId(null)
      }
    }
  }

  const getRepositoryBadge = (_repo: RepositoryResponse) => {
    return <Badge variant="outline">Local</Badge>
  }

  const getExecutionBadge = (repo: RepositoryResponse) => {
    const { codeExecution } = repo
    if (codeExecution.type === 'local') {
      return <Badge variant="secondary">Local</Badge>
    } else {
      return <Badge variant="secondary">Docker</Badge>
    }
  }

  const getRepositoryInfo = (repo: RepositoryResponse) => {
    return repo.path
  }

  const getExecutionInfo = (repo: RepositoryResponse) => {
    const { codeExecution } = repo
    if (codeExecution.type === 'local') {
      return 'Local execution'
    } else {
      return codeExecution.dockerImage || 'Docker'
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex justify-between items-center">
          <div>
            <h2 className="text-2xl font-bold">Repositories</h2>
            <p className="text-muted-foreground">Loading repositories...</p>
          </div>
        </div>
        <div className="grid gap-4">
          {[1, 2, 3].map((i) => (
            <Card key={i} className="animate-pulse">
              <CardContent className="p-6">
                <div className="h-4 bg-muted rounded w-1/4 mb-2"></div>
                <div className="h-3 bg-muted rounded w-1/2"></div>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h2 className="text-2xl font-bold">Environments</h2>
          <p className="text-muted-foreground">
            Manage your development repositories and their configurations
          </p>
        </div>
        <Button onClick={onCreateNew}>Create Repository</Button>
      </div>

      {repositories.length === 0 ? (
        <Card>
          <CardContent className="text-center py-12">
            <div className="text-muted-foreground">
              <p className="text-lg mb-2">No repositories configured</p>
              <p className="text-sm mb-4">Create your first repository to get started</p>
              <Button onClick={onCreateNew}>Create Repository</Button>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4">
          {repositories.map((repository) => (
            <Card key={repository.id}>
              <CardHeader className="pb-3">
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center gap-2 mb-1">
                      <CardTitle className="text-lg">{repository.name}</CardTitle>
                      <div className="flex gap-2">
                        {getRepositoryBadge(repository)}
                        {getExecutionBadge(repository)}
                      </div>
                    </div>
                    {repository.description && (
                      <CardDescription>{repository.description}</CardDescription>
                    )}
                  </div>
                </div>
              </CardHeader>
              <CardContent className="pt-0">
                <div className="space-y-3">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
                    <div>
                      <span className="font-medium text-muted-foreground">Repository:</span>
                      <div className="mt-1">{getRepositoryInfo(repository)}</div>
                    </div>
                    <div>
                      <span className="font-medium text-muted-foreground">Execution:</span>
                      <div className="mt-1">{getExecutionInfo(repository)}</div>
                    </div>
                  </div>


                  <div className="flex items-center justify-between pt-3 border-t">
                    <div className="text-xs text-muted-foreground">
                      Created: {new Date(repository.createdAt).toLocaleDateString()}
                      {repository.updatedAt !== repository.createdAt && (
                        <span className="ml-2">
                          Updated: {new Date(repository.updatedAt).toLocaleDateString()}
                        </span>
                      )}
                    </div>
                    <div className="flex gap-2">
                      <Button variant="outline" size="sm" onClick={() => onEdit(repository)}>
                        Edit
                      </Button>
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => handleDelete(repository.id)}
                        disabled={deletingId === repository.id}
                      >
                        {deletingId === repository.id ? 'Deleting...' : 'Delete'}
                      </Button>
                    </div>
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}
