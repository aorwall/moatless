import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminUsersSidebar } from '@/features/admin'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/users')({
  component: AdminUsersLayout,
})

function AdminUsersLayout() {
  return (
    <ListDetailShell sidebar={<AdminUsersSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
