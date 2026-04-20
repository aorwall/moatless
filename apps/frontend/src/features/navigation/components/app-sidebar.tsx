import { Link } from '@tanstack/react-router'
import { FolderGit2, Home, KeyRound, Wifi, WifiOff } from 'lucide-react'
import { useState } from 'react'
import { useGlobalEventStream } from '@/features/events/contexts/global-event-stream-provider'
import { RepositoriesConfigurationDialog } from '@/features/repositories'
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

import { SidebarTasks } from './sidebar-tasks'

export function AppSidebar() {
  const { isConnected, connectionError } = useGlobalEventStream()
  const [repositoriesConfigOpen, setRepositoriesConfigOpen] = useState(false)

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
                <SidebarMenuButton onClick={() => setRepositoriesConfigOpen(true)}>
                  <FolderGit2 className="h-4 w-4" />
                  <span>Repositories</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton asChild>
                  <Link to="/secrets">
                    <KeyRound className="h-4 w-4" />
                    <span>Secrets</span>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarTasks />
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

      <RepositoriesConfigurationDialog
        open={repositoriesConfigOpen}
        onOpenChange={setRepositoriesConfigOpen}
      />
    </Sidebar>
  )
}
