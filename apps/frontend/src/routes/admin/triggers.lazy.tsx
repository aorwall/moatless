import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminTriggersSidebar } from '@/features/integrations/components/admin-triggers-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/triggers')({
  component: AdminTriggersLayout,
})

function AdminTriggersLayout() {
  return (
    <ListDetailShell sidebar={<AdminTriggersSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
