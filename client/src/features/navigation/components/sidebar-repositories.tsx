import { Link } from '@tanstack/react-router'
import { Plus, Server } from 'lucide-react'

import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { Badge } from '@/lib/components/ui/badge'
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

export function SidebarRepositories() {
  const { data: repositories, isLoading } = useRepositoriesList({
    refetchInterval: 30000, // Refresh every 30 seconds
  })

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Server className="mr-2 h-4 w-4" />
          Repositories
        </SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">Loading repositories...</div>
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  if (!repositories || repositories.length === 0) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Server className="mr-2 h-4 w-4" />
          Repositories
        </SidebarGroupLabel>
        <SidebarGroupAction asChild>
          <Button variant="ghost" size="icon" asChild>
            <Link to="/repositories/new">
              <Plus className="h-4 w-4" />
            </Link>
          </Button>
        </SidebarGroupAction>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">No repositories configured</div>
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  return (
    <SidebarGroup>
      <SidebarGroupLabel asChild>
        <Link to="/repositories" className="hover:text-foreground">
          <Server className="mr-2 h-4 w-4" />
          Repositories
        </Link>
      </SidebarGroupLabel>
      <SidebarGroupAction asChild>
        <Button variant="ghost" size="icon" asChild>
          <Link to="/environments/new">
            <Plus className="h-4 w-4" />
          </Link>
        </Button>
      </SidebarGroupAction>
      <SidebarGroupContent>
        <SidebarMenu>
          {repositories.map((repo) => (
            <SidebarMenuItem key={repo.id}>
              <SidebarMenuButton asChild>
                <Link to="/repositories/$repositoryId" params={{ repositoryId: repo.id }}>
                  <div className="flex flex-col items-start gap-1 overflow-hidden">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm">{repo.name}</span>
                      {repo.is_active && (
                        <Badge variant="secondary" className="h-5 px-1.5 text-xs">
                          Active
                        </Badge>
                      )}
                    </div>
                    {repo.description && (
                      <span className="truncate text-xs text-muted-foreground">
                        {repo.description}
                      </span>
                    )}
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
