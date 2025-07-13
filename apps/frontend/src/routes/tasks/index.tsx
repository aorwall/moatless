import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useEffect } from 'react'
import { z } from 'zod'
import { useGlobalEventStream } from '@/features/events'
import { TaskList } from '@/features/tasks/components/task-list'
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Pagination, PaginationInfo } from '@/lib/components/ui/pagination'

const tasksSearchSchema = z.object({
  page: z.number().min(1).optional().default(1),
  limit: z.number().min(1).max(100).optional().default(20),
  status: z.enum(['open', 'running', 'error', 'merged', 'closed']).optional(),
})

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
  validateSearch: (search) => tasksSearchSchema.parse(search),
})

function TasksIndex() {
  const queryClient = useQueryClient()
  const navigate = useNavigate({ from: '/tasks' })
  const { subscribeToAll } = useGlobalEventStream()
  const searchParams = Route.useSearch()

  const { page = 1, limit = 20, status } = searchParams
  const offset = (page - 1) * limit

  // Fetch tasks with pagination
  const { data, isLoading, error } = useTasksList({ limit, offset, status })

  // Subscribe to global events to invalidate the task list when relevant events occur
  useEffect(() => {
    console.log('[TasksIndex] Setting up global subscription')
    return subscribeToAll((event) => {
      // Invalidate task list on relevant events
      if (
        event.subType &&
        (event.subType === 'task:created' ||
          event.subType === 'task:updated' ||
          event.subType === 'task:statusChanged' ||
          event.subType === 'iteration:created' ||
          event.subType === 'iteration:updated')
      ) {
        queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() })
      }
    })
  }, [subscribeToAll, queryClient])

  const handlePageChange = (newPage: number) => {
    navigate({
      search: (prev) => ({ ...prev, page: newPage }),
    })
  }

  const totalPages = data?.pagination?.total ? Math.ceil(data.pagination.total / limit) : 0

  const totalItems = data?.pagination?.total || 0

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">All Tasks</h1>
          <p className="text-muted-foreground mt-2">
            View and manage all your active and finished tasks
          </p>
        </div>
        {error ? (
          <ErrorAlert error={error} title="Failed to load tasks" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <>
            <TaskList tasks={data?.items || []} loading={false} />
            {totalPages > 1 && (
              <div className="mt-6 flex flex-col items-center gap-4 sm:flex-row sm:justify-between">
                <PaginationInfo currentPage={page} pageSize={limit} totalItems={totalItems} />
                <Pagination
                  currentPage={page}
                  totalPages={totalPages}
                  onPageChange={handlePageChange}
                />
              </div>
            )}
          </>
        )}
      </div>
    </div>
  )
}
