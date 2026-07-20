import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminCommandsSidebar } from '@/features/commands/components/admin-commands-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/commands')({
  component: AdminCommandsLayout,
})

function AdminCommandsLayout() {
  return (
    <ListDetailShell sidebar={<AdminCommandsSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
