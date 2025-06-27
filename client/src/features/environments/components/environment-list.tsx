import type { EnvironmentResponse } from '@shared/types/environment'
import { Link } from '@tanstack/react-router'
import { Plus, Trash2 } from 'lucide-react'
import { useDeleteEnvironment } from '@/features/environments/hooks/use-environments'
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

interface EnvironmentListProps {
  environments?: Array<EnvironmentResponse>
  isLoading: boolean
}

export function EnvironmentList({ environments, isLoading }: EnvironmentListProps) {
  const deleteEnvironment = useDeleteEnvironment()

  const handleDelete = (envId: string, envName: string, event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    if (confirm(`Are you sure you want to delete the environment "${envName}"?`)) {
      deleteEnvironment.mutate(envId)
    }
  }

  if (isLoading) {
    return <div className="text-center py-8 text-muted-foreground">Loading environments...</div>
  }

  if (!environments || environments.length === 0) {
    return (
      <div className="text-center py-8 space-y-4">
        <div className="text-muted-foreground">
          <p>No environments configured yet.</p>
          <p className="text-sm mt-2">Create your first environment to get started.</p>
        </div>
        <Button asChild>
          <Link to="/environments/new">
            <Plus className="h-4 w-4 mr-2" />
            New Environment
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
            <h2 className="text-2xl font-bold">Environments</h2>
            <p className="text-muted-foreground">Manage your agent execution environments</p>
          </div>
          <Button asChild>
            <Link to="/environments/new">
              <Plus className="h-4 w-4 mr-2" />
              New Environment
            </Link>
          </Button>
        </div>
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[30%]">Name</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Repository</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {environments.map((env) => (
                <TableRow key={env.id} className="cursor-pointer">
                  <TableCell className="font-medium">
                    <Link
                      to="/environments/$environmentId"
                      params={{ environmentId: env.id }}
                      className="block hover:text-primary"
                    >
                      {env.name}
                    </Link>
                  </TableCell>
                  <TableCell className="text-muted-foreground">{env.repository.type}</TableCell>
                  <TableCell className="text-muted-foreground max-w-0">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="truncate block">
                          {env.repository.type === 'github'
                            ? env.repository.repoId
                            : env.repository.path}
                        </span>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p className="text-xs">
                          {env.repository.type === 'github'
                            ? env.repository.repoId
                            : env.repository.path}
                        </p>
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-2">
                      <Button variant="ghost" size="sm" asChild>
                        <Link to="/environments/$environmentId" params={{ environmentId: env.id }}>
                          Edit
                        </Link>
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={(e) => handleDelete(env.id, env.name, e)}
                        disabled={deleteEnvironment.isPending}
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
