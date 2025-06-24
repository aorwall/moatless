import { Link } from '@tanstack/react-router';
import { Plus, Server } from 'lucide-react';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';
import { Badge } from '@/lib/components/ui/badge';
import { Button } from '@/lib/components/ui/button';
import {
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar';

export function SidebarEnvironments() {
  const { data: environments, isLoading } = useEnvironmentsList({
    refetchInterval: 30000, // Refresh every 30 seconds
  });

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Server className="mr-2 h-4 w-4" />
          Environments
        </SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">
            Loading environments...
          </div>
        </SidebarGroupContent>
      </SidebarGroup>
    );
  }

  if (!environments || environments.length === 0) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>
          <Server className="mr-2 h-4 w-4" />
          Environments
        </SidebarGroupLabel>
        <SidebarGroupAction asChild>
          <Button variant="ghost" size="icon" asChild>
            <Link to="/environments/new">
              <Plus className="h-4 w-4" />
            </Link>
          </Button>
        </SidebarGroupAction>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-muted-foreground">
            No environments configured
          </div>
        </SidebarGroupContent>
      </SidebarGroup>
    );
  }

  return (
    <SidebarGroup>
      <SidebarGroupLabel asChild>
        <Link to="/environments" className="hover:text-foreground">
          <Server className="mr-2 h-4 w-4" />
          Environments
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
          {environments.map((env) => (
            <SidebarMenuItem key={env.id}>
              <SidebarMenuButton asChild>
                <Link
                  to="/environments/$environmentId"
                  params={{ environmentId: env.id }}
                >
                  <div className="flex flex-col items-start gap-1 overflow-hidden">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm">{env.name}</span>
                      {env.is_active && (
                        <Badge
                          variant="secondary"
                          className="h-5 px-1.5 text-xs"
                        >
                          Active
                        </Badge>
                      )}
                    </div>
                    {env.description && (
                      <span className="truncate text-xs text-muted-foreground">
                        {env.description}
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
  );
}
