import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminWorkspacesSidebar } from '@/features/workspaces/components/admin-workspaces-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/workspaces')({
  component: AdminWorkspacesLayout,
})

function AdminWorkspacesLayout() {
  return (
    <ListDetailShell sidebar={<AdminWorkspacesSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
