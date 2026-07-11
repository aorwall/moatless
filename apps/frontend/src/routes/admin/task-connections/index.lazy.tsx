import { createLazyFileRoute } from '@tanstack/react-router'
import { Link2, Loader2, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { AdminPage } from '@/features/admin'
import { TaskConnectionsTable, useTaskConnections } from '@/features/task-connections'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'

export const Route = createLazyFileRoute('/admin/task-connections/')({
  component: TaskConnectionsPage,
})

function TaskConnectionsPage() {
  const [refresh, setRefresh] = useState(false)
  const { data, isLoading, isFetching, error, refetch } = useTaskConnections({
    limit: 50,
    refresh,
  })
  const rows = data?.items ?? []

  const handleRefresh = () => {
    if (refresh) {
      refetch()
    } else {
      setRefresh(true)
    }
  }

  return (
    <AdminPage
      fullWidth
      title="Task Connections"
      description="Tasks and the external GitHub PRs, Linear issues, and Slack threads they're wired to — with live status and per-connection message counts."
      actions={
        <Button onClick={handleRefresh} size="sm" variant="outline" disabled={isFetching}>
          <RefreshCw className={`mr-2 h-4 w-4 ${isFetching ? 'animate-spin' : ''}`} />
          Refresh status
        </Button>
      }
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : error ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>
            Failed to load task connections. Please try again later.
          </AlertDescription>
        </Alert>
      ) : rows.length === 0 ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <Link2 className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No tasks with connections yet</p>
        </div>
      ) : (
        <TaskConnectionsTable rows={rows} />
      )}
    </AdminPage>
  )
}
