import { Link } from '@tanstack/react-router';
import { Loader2 } from 'lucide-react';

import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons';
import { useTasksList } from '@/features/tasks/hooks/use-tasks';
import {
    SidebarGroup,
    SidebarGroupContent,
    SidebarGroupLabel,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
} from '@/lib/components/ui/sidebar';

export function SidebarTasks() {
  // Fetch all tasks
  const {
    data: tasks,
    isLoading,
    error,
  } = useTasksList(
    {},
    {
      refetchInterval: 5000,
      refetchIntervalInBackground: true,
    },
  );

  // Sort all tasks by last_message_at, with running tasks first
  const sortedTasks =
    tasks
      ?.sort((a, b) => {
        // First sort by is_running
        if (a.is_running && !b.is_running) {
          return -1;
        }
        if (!a.is_running && b.is_running) {
          return 1;
        }
        // Then by last_message_at
        return (
          new Date(b.last_message_at || b.created_at).getTime() -
          new Date(a.last_message_at || a.created_at).getTime()
        );
      })
      .slice(0, 20) || [];

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarTasksSkeleton />
        </SidebarGroupContent>
      </SidebarGroup>
    );
  }

  if (error) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-destructive">
            Failed to load tasks
          </div>
        </SidebarGroupContent>
      </SidebarGroup>
    );
  }

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Tasks</SidebarGroupLabel>
      <SidebarGroupContent>
        {sortedTasks.length > 0 ? (
          <SidebarMenu>
            {sortedTasks.map((task) => (
              <SidebarMenuItem key={task.task_id}>
                <SidebarMenuButton asChild>
                  <Link to="/tasks/$taskId" params={{ taskId: task.task_id }}>
                    <div className="flex flex-col items-start gap-1 overflow-hidden w-full">
                      <div className="flex items-center gap-2 w-full">
                        <span className="truncate text-sm flex-1">
                          {task.title || task.agent_type}
                        </span>
                        {task.is_running && (
                          <Loader2 className="h-3 w-3 animate-spin text-blue-500 flex-shrink-0" />
                        )}
                      </div>
                      <div className="flex items-center gap-2 text-xs text-muted-foreground w-full">
                        <span className="truncate">{task.environment_name}</span>
                        <span className="text-muted-foreground/60">•</span>
                        <span className="flex-shrink-0">
                          {new Date(
                            task.last_message_at || task.created_at,
                          ).toLocaleDateString()}
                        </span>
                      </div>
                    </div>
                  </Link>
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        ) : (
          <div className="px-3 py-2 text-sm text-muted-foreground">
            No tasks found
          </div>
        )}
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
