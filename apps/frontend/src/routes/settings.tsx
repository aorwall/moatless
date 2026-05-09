import { createFileRoute, Outlet } from '@tanstack/react-router'
import { SettingsSidebar } from '@/features/settings/components/settings-sidebar'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

export const Route = createFileRoute('/settings')({
  component: SettingsLayout,
})

function SettingsLayout() {
  return (
    <SidebarProvider>
      <div className="flex h-dvh w-full">
        <SettingsSidebar />
        <div className="flex-1 overflow-auto">
          <Outlet />
        </div>
      </div>
    </SidebarProvider>
  )
}
