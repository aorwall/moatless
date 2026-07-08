import { Link } from '@tanstack/react-router'
import { Wifi, WifiOff } from 'lucide-react'
import { useGlobalEventStream } from '@/features/events/contexts/global-event-stream-provider'
import {
  Sidebar,
  SidebarContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'

import { SidebarTasks } from './sidebar-tasks'

export function AppSidebar() {
  const { isConnected, connectionError } = useGlobalEventStream()

  return (
    <Sidebar>
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" asChild>
              <Link to="/">
                <span className="font-bold">Moatless Vibe</span>
                {isConnected ? (
                  <div title="Connected to event stream">
                    <Wifi className="h-4 w-4 text-green-500" />
                  </div>
                ) : (
                  <div title={connectionError?.message || 'Disconnected from event stream'}>
                    <WifiOff className="h-4 w-4 text-red-500" />
                  </div>
                )}
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        <SidebarTasks />
      </SidebarContent>
    </Sidebar>
  )
}
