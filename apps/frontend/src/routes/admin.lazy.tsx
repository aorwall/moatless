import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminSidebar } from '@/features/admin'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

export const Route = createLazyFileRoute('/admin')({
  component: AdminLayout,
})

function AdminLayout() {
  return (
    <SidebarProvider>
      <div className="flex h-full w-full">
        <AdminSidebar />
        <div className="flex-1 overflow-auto">
          <Outlet />
        </div>
      </div>
    </SidebarProvider>
  )
}
