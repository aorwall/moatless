import type { TaskStatus } from '@shared/types/api'
import { Link } from '@tanstack/react-router'
import { CheckCircle, CircleDot, Loader2 } from 'lucide-react'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

const getStatusConfig = (status: TaskStatus) => {
  switch (status) {
    case 'active':
      return {
        icon: <Loader2 className="h-3 w-3 animate-spin text-blue-600 dark:text-blue-400" />,
        tooltip: 'In Progress',
      }
    case 'completed':
      return {
        icon: <CheckCircle className="h-3 w-3 text-green-600 dark:text-green-400" />,
        tooltip: 'Completed',
      }
    case 'error':
      return {
        icon: <CircleDot className="h-3 w-3 text-red-600 dark:text-red-400" />,
        tooltip: 'Error',
      }
    case 'closed':
      return {
        icon: <CircleDot className="h-3 w-3 text-gray-600 dark:text-gray-400" />,
        tooltip: 'Closed',
      }
    default:
      return {
        icon: <CircleDot className="h-3 w-3 text-gray-600 dark:text-gray-400" />,
        tooltip: 'Unknown',
      }
  }
}

function getRelativeTime(date: Date | string): string {
  const now = new Date()
  const then = new Date(date)
  const seconds = Math.floor((now.getTime() - then.getTime()) / 1000)

  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d ago`
  const weeks = Math.floor(days / 7)
  if (weeks < 4) return `${weeks}w ago`
  const months = Math.floor(days / 30)
  if (months < 12) return `${months}mo ago`
  const years = Math.floor(days / 365)
  return `${years}y ago`
}

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
  )

  // Sort all tasks by last_message_at, with active tasks first
  const sortedTasks =
    tasks
      ?.sort((a, b) => {
        // First sort by active status
        if (a.status === 'active' && b.status !== 'active') {
          return -1
        }
        if (a.status !== 'active' && b.status === 'active') {
          return 1
        }
        // Then by last_message_at
        return (
          new Date(b.lastMessageAt || b.createdAt).getTime() -
          new Date(a.lastMessageAt || a.createdAt).getTime()
        )
      })
      .slice(0, 20) || []

  if (isLoading) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarTasksSkeleton />
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  if (error) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          <div className="px-3 py-2 text-sm text-destructive">Failed to load tasks</div>
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  return (
    <TooltipProvider>
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          {sortedTasks.length > 0 ? (
            <SidebarMenu>
              {sortedTasks.map((task) => {
                const statusConfig = getStatusConfig(task.status)
                return (
                  <SidebarMenuItem key={task.taskId}>
                    <SidebarMenuButton className="h-auto p-2 hover:bg-sidebar-accent" asChild>
                      <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
                        <div className="flex w-full items-center gap-2 min-w-0">
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <div className="flex items-center">{statusConfig.icon}</div>
                            </TooltipTrigger>
                            <TooltipContent side="right" className="text-xs">
                              {statusConfig.tooltip}
                            </TooltipContent>
                          </Tooltip>
                          <div className="flex flex-col min-w-0 flex-1 gap-0.5">
                            <span className="font-medium text-sm truncate leading-tight">
                              {task.agentType}
                            </span>
                            <div className="flex items-center justify-between text-xs text-sidebar-foreground/60">
                              <span className="truncate">{task.environmentId}</span>
                              <span className="shrink-0">
                                {getRelativeTime(task.lastMessageAt || task.createdAt)}
                              </span>
                            </div>
                          </div>
                        </div>
                      </Link>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                )
              })}
            </SidebarMenu>
          ) : (
            <div className="px-3 py-2 text-sm text-muted-foreground">No tasks found</div>
          )}
        </SidebarGroupContent>
      </SidebarGroup>
    </TooltipProvider>
  )
}
