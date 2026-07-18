import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminConnectionsSidebar } from '@/features/integrations/components/admin-connections-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/integrations/connections')({
  component: AdminConnectionsLayout,
})

function AdminConnectionsLayout() {
  return (
    <ListDetailShell sidebar={<AdminConnectionsSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
