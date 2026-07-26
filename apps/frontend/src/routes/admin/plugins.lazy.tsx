import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminPluginsSidebar } from '@/features/plugins/components/admin-plugins-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/plugins')({
  component: AdminPluginsLayout,
})

function AdminPluginsLayout() {
  return (
    <ListDetailShell sidebar={<AdminPluginsSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
