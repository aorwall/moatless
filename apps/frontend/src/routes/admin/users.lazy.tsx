import { createLazyFileRoute, Outlet } from '@tanstack/react-router'
import { AdminUsersSidebar } from '@/features/admin'

export const Route = createLazyFileRoute('/admin/users')({
  component: AdminUsersLayout,
})

function AdminUsersLayout() {
  return (
    <div className="flex h-full min-h-0 w-full">
      <AdminUsersSidebar />
      <div className="min-w-0 flex-1 overflow-auto">
        <Outlet />
      </div>
    </div>
  )
}
