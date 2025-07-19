import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { useState } from 'react'
import {
  RepositoriesConfigurationDialog,
  useRepository,
  VerificationDialog,
} from '@/features/repositories'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Skeleton } from '@/lib/components/ui/skeleton'

export const Route = createFileRoute('/repositories/$repositoryId')({
  component: RepositoryDetail,
})

function RepositoryDetail() {
  const navigate = useNavigate()
  const { repositoryId } = Route.useParams()
  const [configOpen, setConfigOpen] = useState(false)

  const { data: repository, isLoading, error } = useRepository(repositoryId)

  if (isLoading) {
    return (
      <div className="container mx-auto py-6">
        <div className="max-w-4xl mx-auto space-y-4">
          <Skeleton className="h-32 w-full" />
          <div className="grid gap-4 md:grid-cols-3">
            <Skeleton className="h-32 w-full" />
            <Skeleton className="h-32 w-full" />
            <Skeleton className="h-32 w-full" />
          </div>
        </div>
      </div>
    )
  }

  if (error || !repository) {
    return (
      <div className="container mx-auto py-6">
        <div className="max-w-4xl mx-auto text-center">
          <p className="text-destructive">{error ? error.message : 'Repository not found'}</p>
        </div>
      </div>
    )
  }

  const handleCloseConfig = (open: boolean) => {
    setConfigOpen(open)
    // Navigate back to list when dialog closes
    if (!open) {
      navigate({ to: '/repositories' })
    }
  }

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-4xl mx-auto space-y-6">
        <div className="flex justify-between items-center">
          <div>
            <h1 className="text-2xl font-bold">{repository.name}</h1>
            {repository.description && (
              <p className="text-muted-foreground mt-1">{repository.description}</p>
            )}
          </div>
          <div className="flex gap-2">
            <VerificationDialog repositoryId={repositoryId} />
            <Button onClick={() => setConfigOpen(true)}>
              <Settings className="h-4 w-4 mr-2" />
              Configure
            </Button>
          </div>
        </div>

        {/* Simple summary cards */}
        <div className="grid gap-4 md:grid-cols-3">
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Repository Path</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm text-muted-foreground">{repository.path}</p>
              {repository.useGitWorktrees && (
                <Badge variant="secondary" className="mt-2">
                  Git Worktrees
                </Badge>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Code Execution</CardTitle>
            </CardHeader>
            <CardContent>
              <Badge
                variant={repository.codeExecution?.type === 'docker' ? 'default' : 'secondary'}
              >
                {repository.codeExecution?.type || 'Not configured'}
              </Badge>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">Preview Environment</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm">{repository.previewConfig?.framework || 'Not configured'}</p>
            </CardContent>
          </Card>
        </div>

        <RepositoriesConfigurationDialog
          open={configOpen}
          onOpenChange={handleCloseConfig}
          initialRepositoryId={repositoryId}
        />
      </div>
    </div>
  )
}
