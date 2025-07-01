import { Link } from '@tanstack/react-router'
import { Command, Plus } from 'lucide-react'

import { useCommandsList } from '@/features/commands/hooks/use-commands'
import { Button } from '@/lib/components/ui/button'
import {
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'

export function SidebarCommands() {
  const { data: commands, isLoading } = useCommandsList({
    refetchInterval: 30000, // Refresh every 30 seconds
  })

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Command className="mr-2 h-4 w-4" />
          Commands
        </SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">Loading commands...</div>
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  if (!commands || commands.length === 0) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Command className="mr-2 h-4 w-4" />
          Commands
        </SidebarGroupLabel>
        <SidebarGroupAction asChild>
          <Button variant="ghost" size="icon" asChild>
            <Link to="/commands/new">
              <Plus className="h-4 w-4" />
            </Link>
          </Button>
        </SidebarGroupAction>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">No commands configured</div>
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  return (
    <SidebarGroup>
      <SidebarGroupLabel asChild>
        <Link to="/commands" className="hover:text-foreground">
          <Command className="mr-2 h-4 w-4" />
          Commands
        </Link>
      </SidebarGroupLabel>
      <SidebarGroupAction asChild>
        <Button variant="ghost" size="icon" asChild>
          <Link to="/commands/new">
            <Plus className="h-4 w-4" />
          </Link>
        </Button>
      </SidebarGroupAction>
      <SidebarGroupContent>
        <SidebarMenu>
          {commands.map((command) => (
            <SidebarMenuItem key={command.name}>
              <SidebarMenuButton asChild>
                <Link to="/commands/$commandName" params={{ commandName: command.name }}>
                  <div className="flex flex-col items-start gap-1 overflow-hidden">
                    <span className="truncate text-sm">{command.name}</span>
                    <span className="truncate text-xs text-muted-foreground">
                      {command.description}
                    </span>
                  </div>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  )
}
