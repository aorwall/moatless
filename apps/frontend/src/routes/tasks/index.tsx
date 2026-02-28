import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useCallback } from 'react'
import { z } from 'zod'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { TaskDashboard } from '@/features/tasks/components/dashboard'
import type { DashboardSearchParams } from '@/features/tasks/components/dashboard/task-filters'

const tasksSearchSchema = z.object({
  view: z.enum(['cards', 'table']).optional().catch(undefined),
  tag: z.string().optional().catch(undefined),
  createdBy: z.string().optional().catch(undefined),
  repositoryId: z.string().optional().catch(undefined),
  status: z.enum(['all', 'open', 'error', 'closed']).optional().catch(undefined),
  page: z.coerce.number().min(1).optional().catch(undefined),
})

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
  validateSearch: tasksSearchSchema,
})

function TasksIndex() {
  const search = Route.useSearch()
  const navigate = useNavigate({ from: '/tasks/' })
  const { user } = useAuth()

  const handleSearchChange = useCallback(
    (params: DashboardSearchParams) => {
      navigate({ search: params })
    },
    [navigate],
  )

  return (
    <TaskDashboard
      view={search.view ?? 'cards'}
      tag={search.tag}
      createdBy={search.createdBy === 'all' ? undefined : (search.createdBy ?? user?.id)}
      repositoryId={search.repositoryId}
      status={search.status === 'all' ? undefined : (search.status ?? 'open')}
      page={search.page ?? 1}
      onSearchChange={handleSearchChange}
    />
  )
}
