import { useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useGlobalEventStream } from '@/features/events'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { queryKeys } from '@/lib/api/query-keys'
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
  const queryClient = useQueryClient()
  const { subscribeToAll } = useGlobalEventStream()

  // Fetch all non-closed tasks
  const {
    data: tasks,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList(
    {},
    {
      // Remove polling since we're using global event stream
      refetchInterval: false,
    },
  )

  // Subscribe to global events to invalidate the task list when relevant events occur
  useEffect(() => {
    return subscribeToAll((event) => {
      // Invalidate task list on relevant events
      if (
        event.type === 'task:created' ||
        event.type === 'task:updated' ||
        event.type === 'task:statusChanged' ||
        event.type === 'message' ||
        event.type === 'iteration:created' ||
        event.type === 'iteration:updated'
      ) {
        queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() })
      }
    })
  }, [subscribeToAll, queryClient])

  // Fetch repositories to map IDs to names
  const { data: repositories } = useRepositoriesList()

  // Filter out closed tasks and sort by last_message_at, with open/running tasks first
  const sortedTasks =
    tasks
      ?.filter((task) => task.status !== 'closed') // Filter out closed tasks
      ?.sort((a, b) => {
        // First sort by active status (open/running tasks come first)
        const aActive = a.status === 'open' || a.status === 'running'
        const bActive = b.status === 'open' || b.status === 'running'

        if (aActive && !bActive) {
          return -1
        }
        if (!aActive && bActive) {
          return 1
        }
        // Then by last_message_at
        return (
          new Date(b.lastMessageAt || b.createdAt).getTime() -
          new Date(a.lastMessageAt || a.createdAt).getTime()
        )
      })
      .slice(0, 20) || []

  // Create a map of repository IDs to names
  const repositoryMap =
    repositories?.reduce(
      (acc, env) => {
        acc[env.id] = env.name
        return acc
      },
      {} as Record<string, string>,
    ) || {}

  if (isLoadingTasks) {
    return (
      <SidebarGroup>
        <SidebarGroupLabel>Tasks</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarTasksSkeleton />
        </SidebarGroupContent>
      </SidebarGroup>
    )
  }

  if (tasksError) {
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
                const statusConfig = getTaskStatusConfig(task.status)
                // Use the initial message from the first iteration as the title
                const taskTitle =
                  task.iterations?.[0]?.initialMessage?.split('\n')[0] || task.agentType
                const repositoryName = repositoryMap[task.repositoryId] || task.repositoryId

                return (
                  <SidebarMenuItem key={task.taskId}>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <SidebarMenuButton className="h-auto p-2 hover:bg-sidebar-accent" asChild>
                          <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
                            <div className="flex w-full items-center gap-2 min-w-0">
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <div className={`flex items-center ${statusConfig.color}`}>
                                    {statusConfig.icon}
                                  </div>
                                </TooltipTrigger>
                                <TooltipContent side="right" className="text-xs">
                                  {statusConfig.tooltip}
                                </TooltipContent>
                              </Tooltip>
                              <div className="flex flex-col min-w-0 flex-1 gap-0.5">
                                <span className="font-medium text-sm truncate leading-tight">
                                  {taskTitle}
                                </span>
                                <div className="flex items-center justify-between text-xs text-sidebar-foreground/60">
                                  <span className="truncate">{repositoryName}</span>
                                  <span className="shrink-0">
                                    {getRelativeTime(task.lastMessageAt || task.createdAt)}
                                  </span>
                                </div>
                              </div>
                            </div>
                          </Link>
                        </SidebarMenuButton>
                      </TooltipTrigger>
                      <TooltipContent side="right" className="max-w-xs">
                        <div className="space-y-1">
                          <p className="font-medium text-sm break-words">
                            {taskTitle.length > 150
                              ? `${taskTitle.substring(0, 150)}...`
                              : taskTitle}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {repositoryName} •{' '}
                            {getRelativeTime(task.lastMessageAt || task.createdAt)}
                          </p>
                        </div>
                      </TooltipContent>
                    </Tooltip>
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
