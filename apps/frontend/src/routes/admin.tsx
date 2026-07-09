import type { SessionResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { meHandler } from '@moatless/api-client/sandbox/auth/auth'
import { createFileRoute, Outlet, redirect } from '@tanstack/react-router'
import { AdminSidebar } from '@/features/admin'
import { queryClient } from '@/lib/api/query-client'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

export const Route = createFileRoute('/admin')({
  beforeLoad: async () => {
    const session = await queryClient.ensureQueryData({
      queryKey: ['auth', 'session'],
      queryFn: () => unwrapResponse<SessionResponse>(meHandler()),
      staleTime: 5 * 60 * 1000,
    })
    if (session?.user?.role !== 'admin') {
      throw redirect({ to: '/' })
    }
  },
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
