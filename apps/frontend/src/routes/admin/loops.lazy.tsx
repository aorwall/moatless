import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminLoopsSidebar } from '@/features/loops/components/admin-loops-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/loops')({
  component: AdminLoopsLayout,
})

function AdminLoopsLayout() {
  return (
    <ListDetailShell sidebar={<AdminLoopsSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
