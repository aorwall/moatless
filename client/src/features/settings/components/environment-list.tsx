import type { EnvironmentResponse } from '@shared/types/environmenttypes/environment'
import { useState } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Switch } from '@/lib/components/ui/switch'

interface EnvironmentListProps {
  environments: Array<EnvironmentResponse>
  onCreateNew: () => void
  onEdit: (environment: EnvironmentResponse) => void
  onDelete: (environmentId: string) => void
  onToggleActive: (environmentId: string, isActive: boolean) => void
  isLoading?: boolean
}

export function EnvironmentList({
  environments,
  onCreateNew,
  onEdit,
  onDelete,
  onToggleActive,
  isLoading = false,
}: EnvironmentListProps) {
  const [deletingId, setDeletingId] = useState<string | null>(null)

  const handleDelete = async (environmentId: string) => {
    if (
      confirm('Are you sure you want to delete this environment? This action cannot be undone.')
    ) {
      setDeletingId(environmentId)
      try {
        await onDelete(environmentId)
      } finally {
        setDeletingId(null)
      }
    }
  }

  const getRepositoryBadge = (environment: EnvironmentResponse) => {
    const { repository } = environment
    if (repository.type === 'local') {
      return <Badge variant="outline">Local</Badge>
    } else {
      return <Badge variant="outline">GitHub</Badge>
    }
  }

  const getExecutionBadge = (environment: EnvironmentResponse) => {
    const { code_execution } = environment
    if (code_execution.type === 'local') {
      return <Badge variant="secondary">Local</Badge>
    } else {
      return <Badge variant="secondary">Docker</Badge>
    }
  }

  const getRepositoryInfo = (environment: EnvironmentResponse) => {
    const { repository } = environment
    if (repository.type === 'local') {
      return repository.path
    } else {
      return `${repository.repoId}${repository.branch !== 'main' ? ` (${repository.branch})` : ''}`
    }
  }

  const getExecutionInfo = (environment: EnvironmentResponse) => {
    const { code_execution } = environment
    if (code_execution.type === 'local') {
      return 'Local execution'
    } else {
      return code_execution.image || 'Docker'
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex justify-between items-center">
          <div>
            <h2 className="text-2xl font-bold">Environments</h2>
            <p className="text-muted-foreground">Loading environments...</p>
          </div>
        </div>
        <div className="grid gap-4">
          {[1, 2, 3].map((i) => (
            <Card key={i} className="animate-pulse">
              <CardContent className="p-6">
                <div className="h-4 bg-gray-200 rounded w-1/4 mb-2"></div>
                <div className="h-3 bg-gray-200 rounded w-1/2"></div>
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
            Manage your development environments and their configurations
          </p>
        </div>
        <Button onClick={onCreateNew}>Create Environment</Button>
      </div>

      {environments.length === 0 ? (
        <Card>
          <CardContent className="text-center py-12">
            <div className="text-muted-foreground">
              <p className="text-lg mb-2">No environments configured</p>
              <p className="text-sm mb-4">Create your first environment to get started</p>
              <Button onClick={onCreateNew}>Create Environment</Button>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4">
          {environments.map((environment) => (
            <Card key={environment.id} className={!environment.is_active ? 'opacity-60' : ''}>
              <CardHeader className="pb-3">
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center gap-2 mb-1">
                      <CardTitle className="text-lg">{environment.name}</CardTitle>
                      <div className="flex gap-2">
                        {getRepositoryBadge(environment)}
                        {getExecutionBadge(environment)}
                      </div>
                    </div>
                    {environment.description && (
                      <CardDescription>{environment.description}</CardDescription>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-muted-foreground">Active</span>
                      <Switch
                        checked={environment.is_active}
                        onCheckedChange={(checked) => onToggleActive(environment.id, checked)}
                      />
                    </div>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="pt-0">
                <div className="space-y-3">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
                    <div>
                      <span className="font-medium text-muted-foreground">Repository:</span>
                      <div className="mt-1">{getRepositoryInfo(environment)}</div>
                    </div>
                    <div>
                      <span className="font-medium text-muted-foreground">Execution:</span>
                      <div className="mt-1">{getExecutionInfo(environment)}</div>
                    </div>
                  </div>

                  {Object.keys(environment.variables).length > 0 && (
                    <div>
                      <span className="font-medium text-muted-foreground text-sm">Variables:</span>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {Object.keys(environment.variables).map((key) => (
                          <Badge key={key} variant="outline" className="text-xs">
                            {key}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  <div className="flex items-center justify-between pt-3 border-t">
                    <div className="text-xs text-muted-foreground">
                      Created: {new Date(environment.created_at).toLocaleDateString()}
                      {environment.updated_at !== environment.created_at && (
                        <span className="ml-2">
                          Updated: {new Date(environment.updated_at).toLocaleDateString()}
                        </span>
                      )}
                    </div>
                    <div className="flex gap-2">
                      <Button variant="outline" size="sm" onClick={() => onEdit(environment)}>
                        Edit
                      </Button>
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => handleDelete(environment.id)}
                        disabled={deletingId === environment.id}
                      >
                        {deletingId === environment.id ? 'Deleting...' : 'Delete'}
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
