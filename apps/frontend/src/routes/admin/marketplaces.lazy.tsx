import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminMarketplacesSidebar } from '@/features/plugins/components/admin-marketplaces-sidebar'
import { ListDetailShell } from '@/lib/components/ui/list-detail-shell'

export const Route = createLazyFileRoute('/admin/marketplaces')({
  component: AdminMarketplacesLayout,
})

function AdminMarketplacesLayout() {
  return (
    <ListDetailShell sidebar={<AdminMarketplacesSidebar />}>
      <Outlet />
    </ListDetailShell>
  )
}
