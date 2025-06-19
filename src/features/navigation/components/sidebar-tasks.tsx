import { Link } from '@tanstack/react-router';
import { Activity, Clock } from 'lucide-react';

import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons';
import { useTasksList } from '@/features/tasks/hooks/use-tasks';
import { Badge } from '@/lib/components/ui/badge';
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

  // Separate active and recent tasks
  const activeTasks =
    tasks?.filter((t) => t.status === 'active').slice(0, 10) || [];
  const recentTasks =
    tasks
      ?.filter((s) => s.status !== 'active')
      .sort(
        (a, b) =>
          new Date(b.last_message_at).getTime() -
          new Date(a.last_message_at).getTime(),
      )
      .slice(0, 10) || [];

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
    <>
      {/* Active Tasks */}
      {activeTasks.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>
            <Activity className="mr-2 h-4 w-4" />
            Active Tasks
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {activeTasks.map((task) => (
                <SidebarMenuItem key={task.task_id}>
                  <SidebarMenuButton asChild>
                    <Link to={`/tasks/${task.task_id}`}>
                      <div className="flex flex-col items-start gap-1 overflow-hidden">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm">
                            {task.agent_type}
                          </span>
                          <Badge
                            variant="outline"
                            className="h-5 px-1.5 text-xs"
                          >
                            Active
                          </Badge>
                        </div>
                        <span className="truncate text-xs text-muted-foreground">
                          {task.task_id.slice(0, 8)}
                        </span>
                      </div>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      {/* Recent Tasks */}
      {recentTasks.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>
            <Clock className="mr-2 h-4 w-4" />
            Recent Tasks
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {recentTasks.map((task) => (
                <SidebarMenuItem key={task.task_id}>
                  <SidebarMenuButton asChild>
                    <Link to={`/tasks/${task.task_id}`}>
                      <div className="flex flex-col items-start gap-1 overflow-hidden">
                        <span className="truncate text-sm">
                          {task.agent_type}
                        </span>
                        <span className="truncate text-xs text-muted-foreground">
                          {new Date(task.last_message_at).toLocaleDateString()}
                        </span>
                      </div>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      {/* Empty state */}
      {activeTasks.length === 0 && recentTasks.length === 0 && (
        <SidebarGroup>
          <SidebarGroupLabel>Tasks</SidebarGroupLabel>
          <SidebarGroupContent>
            <div className="px-3 py-2 text-sm text-muted-foreground">
              No tasks found
            </div>
          </SidebarGroupContent>
        </SidebarGroup>
      )}
    </>
  );
}
