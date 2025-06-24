import type { EnvironmentResponse } from '@shared/types/environmenttypes/environment'
import { Link } from '@tanstack/react-router'
import { Plus } from 'lucide-react'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Skeleton } from '@/lib/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'

interface EnvironmentListProps {
  environments?: Array<EnvironmentResponse>
  isLoading: boolean
}

export function EnvironmentList({ environments, isLoading }: EnvironmentListProps) {
  if (isLoading) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Environments</CardTitle>
          <CardDescription>Manage your agent execution environments</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-3">
            {[1, 2, 3].map((i) => (
              <Skeleton key={i} className="h-12 w-full" />
            ))}
          </div>
        </CardContent>
      </Card>
    )
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle>Environments</CardTitle>
            <CardDescription>Manage your agent execution environments</CardDescription>
          </div>
          <Button size="sm" asChild>
            <Link to="/environments/new">
              <Plus className="h-4 w-4 mr-2" />
              New Environment
            </Link>
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {!environments || environments.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            <p>No environments configured yet.</p>
            <p className="text-sm mt-2">Create your first environment to get started.</p>
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Repository URL</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {environments.map((env) => (
                <TableRow key={env.id}>
                  <TableCell className="font-medium">{env.name}</TableCell>
                  <TableCell>{env.repository.type}</TableCell>
                  <TableCell className="max-w-xs truncate">
                    {env.repository.type === 'github' ? env.repository.repoId : env.repository.path}
                  </TableCell>
                  <TableCell>
                    <Badge variant={env.is_active ? 'default' : 'secondary'}>
                      {env.is_active ? 'Active' : 'Inactive'}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="sm" asChild>
                      <Link to="/environments/$environmentId" params={{ environmentId: env.id }}>
                        Edit
                      </Link>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  )
}
