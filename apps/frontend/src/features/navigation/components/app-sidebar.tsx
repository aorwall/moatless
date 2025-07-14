import { Link } from '@tanstack/react-router'
import { Home, Settings, Wifi, WifiOff } from 'lucide-react'
import { useGlobalEventStream } from '@/features/events/contexts/global-event-stream-provider'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'

import { SidebarCommands } from './sidebar-commands'
import { SidebarRepositories } from './sidebar-repositories'
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
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton asChild>
                  <Link to="/">
                    <Home className="h-4 w-4" />
                    <span>Dashboard</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton asChild>
                  <Link to="/settings">
                    <Settings className="h-4 w-4" />
                    <span>Settings</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarTasks />
        <SidebarRepositories />
        <SidebarCommands />
      </SidebarContent>

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton variant="outline" size="sm" className="text-xs">
              v1.0.0
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  )
}
