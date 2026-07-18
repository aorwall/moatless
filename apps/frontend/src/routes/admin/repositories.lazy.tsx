import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminRepositoriesSidebar } from '@/features/repositories/components/admin-repositories-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/repositories')({
  component: AdminRepositoriesLayout,
})

function AdminRepositoriesLayout() {
  return (
    <ListDetailShell sidebar={<AdminRepositoriesSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
