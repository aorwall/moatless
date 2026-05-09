import type { RepositoryResponse } from '@moatless/api-client'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { Loader2, Pencil, Plus, Route as RouteIcon } from 'lucide-react'
import { useMemo } from 'react'
import { AdminPage } from '@/features/admin'
import { useTaskRoutes } from '@/features/integrations/hooks/use-integrations'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'

const ROUTING_MODE_LABELS: Record<string, string> = {
  by_subject: 'By Subject',
  ongoing: 'Ongoing',
}

function getRoutingModeLabel(mode: string): string {
  return ROUTING_MODE_LABELS[mode] ?? mode
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  return date.toLocaleString()
}

function createRepositoryMap(repositories: RepositoryResponse[] | undefined) {
  return new Map((repositories ?? []).map((repository) => [repository.id, repository.name]))
}

export const Route = createFileRoute('/admin/integrations/routes/')({
  component: RoutesListPage,
})

function RoutesListPage() {
  const navigate = useNavigate()
  const { data: routes, isLoading: routesLoading, error: routesError } = useTaskRoutes()
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList()

  const items = routes ?? []
  const repositoryItems = repositories ?? []
  const repositoriesById = useMemo(() => createRepositoryMap(repositories), [repositories])
  const isLoading = routesLoading || repositoriesLoading
  const hasRepositories = repositoryItems.some((repo) => repo.kind !== 'template')

  return (
    <AdminPage
      title="Routes"
      description="Reusable task routes that triggers send messages to."
      actions={
        <Button
          onClick={() => navigate({ to: '/admin/integrations/routes/new' })}
          size="sm"
          disabled={!hasRepositories}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Route
        </Button>
      }
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : routesError ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>Failed to load routes. Please try again later.</AlertDescription>
        </Alert>
      ) : items.length === 0 ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <RouteIcon className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No task routes yet</p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Repository</TableHead>
              <TableHead>Routing Mode</TableHead>
              <TableHead>Task Name</TableHead>
              <TableHead>Tags</TableHead>
              <TableHead>Prompt Template</TableHead>
              <TableHead>Created</TableHead>
              <TableHead>Updated</TableHead>
              <TableHead className="w-12" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {items.map((route) => (
              <TableRow key={route.id}>
                <TableCell className="font-medium">
                  {repositoriesById.get(route.repositoryId) ?? route.repositoryId}
                </TableCell>
                <TableCell>{getRoutingModeLabel(route.routingMode)}</TableCell>
                <TableCell>{route.taskName || '-'}</TableCell>
                <TableCell>{route.tagIds.length}</TableCell>
                <TableCell>
                  <Badge variant={route.promptTemplate ? 'secondary' : 'outline'}>
                    {route.promptTemplate ? 'Configured' : 'None'}
                  </Badge>
                </TableCell>
                <TableCell>{formatTimestamp(route.createdAt)}</TableCell>
                <TableCell>{formatTimestamp(route.updatedAt)}</TableCell>
                <TableCell>
                  <Button variant="ghost" size="icon" asChild title="Edit">
                    <Link
                      to="/admin/integrations/routes/$route-id"
                      params={{ 'route-id': route.id }}
                    >
                      <Pencil className="h-4 w-4" />
                    </Link>
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </AdminPage>
  )
}
