import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminTeamsSidebar } from '@/features/admin/components/admin-teams-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/teams')({
  component: AdminTeamsLayout,
})

function AdminTeamsLayout() {
  return (
    <ListDetailShell sidebar={<AdminTeamsSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
