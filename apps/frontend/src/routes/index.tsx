import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useCallback, useState } from 'react'
import { z } from 'zod'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { TopBar } from '@/features/navigation/components/top-bar'
import { SettingsDialog } from '@/features/settings'
import { TaskDashboard } from '@/features/tasks/components/dashboard'
import type { DashboardSearchParams } from '@/features/tasks/components/dashboard/task-filters'

const indexSearchSchema = z.object({
  view: z.enum(['cards', 'table']).optional().catch(undefined),
  tag: z.string().optional().catch(undefined),
  createdBy: z.string().optional().catch(undefined),
  repositoryId: z.string().optional().catch(undefined),
  status: z.enum(['all', 'open', 'error', 'closed']).optional().catch(undefined),
  page: z.coerce.number().min(1).optional().catch(undefined),
})

export const Route = createFileRoute('/')({
  component: Dashboard,
  validateSearch: indexSearchSchema,
})

function Dashboard() {
  const [settingsOpen, setSettingsOpen] = useState(false)
  const search = Route.useSearch()
  const navigate = useNavigate({ from: '/' })
  const { user } = useAuth()

  const handleSearchChange = useCallback(
    async (params: DashboardSearchParams) => {
      await navigate({ search: params })
    },
    [navigate],
  )

  return (
    <div className="flex flex-col h-full">
      <TopBar
        onRepositoriesClick={() => navigate({ to: '/admin/repositories' })}
        onSettingsClick={() => setSettingsOpen(true)}
      />
      <div className="flex-1 overflow-auto">
        <TaskDashboard
          view={search.view ?? 'cards'}
          tag={search.tag}
          createdBy={search.createdBy ?? user?.id}
          repositoryId={search.repositoryId}
          status={search.status ?? 'open'}
          page={search.page ?? 1}
          onSearchChange={handleSearchChange}
        />
      </div>
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </div>
  )
}
