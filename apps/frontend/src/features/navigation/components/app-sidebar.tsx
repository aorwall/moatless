import { Link } from '@tanstack/react-router'
import { CreateTaskButton } from '@/features/tasks/components/compose/create-task-button'
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
  return (
    <Sidebar>
      <SidebarHeader>
        <div className="flex items-center justify-between gap-2">
          <SidebarMenu className="flex-1">
            <SidebarMenuItem>
              <SidebarMenuButton size="lg" asChild>
                <Link to="/">
                  <span className="font-bold">Moatless Vibe</span>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
          <CreateTaskButton className="shrink-0" />
        </div>
      </SidebarHeader>

      <SidebarContent>
        <SidebarTasks />
      </SidebarContent>
    </Sidebar>
  )
}
