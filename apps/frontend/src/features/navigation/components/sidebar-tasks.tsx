import type { Task } from '@moatless/types/types/task'
import { Link } from '@tanstack/react-router'
import { Plus } from 'lucide-react'
import { useState } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { CreateTaskDialog } from '@/features/tasks/components/create-task-dialog'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import {
  formatGitSyncDetails,
  getGitSyncStatusConfig,
} from '@/features/tasks/utils/git-sync-status'
import { Button } from '@/lib/components/ui/button'
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

// Component for rendering individual task item with git sync status
function SidebarTaskItem({
  task,
  repositoryName,
}: {
  task: any // Using any to match existing code style
  repositoryName: string
}) {
  const taskTitle = task.iterations?.[0]?.initialMessage?.split('\n')[0] || task.agentType

  // Get git sync status from the last iteration for tooltip
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  const gitSyncStatus = task.status === 'open' ? lastIteration?.gitSyncStatus : null
  const gitSyncConfig = gitSyncStatus ? getGitSyncStatusConfig(gitSyncStatus.status) : null

  return (
    <SidebarMenuItem>
      <Tooltip>
        <TooltipTrigger asChild>
          <SidebarMenuButton className="h-auto p-2 hover:bg-sidebar-accent" asChild>
            <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
              <div className="flex w-full items-center gap-2 min-w-0">
                <div className="flex items-center gap-1">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <div>
                        <TaskStatusIcon task={task} showText={false} />
                      </div>
                    </TooltipTrigger>
                    <TooltipContent side="right" className="text-xs">
                      {task.status === 'open' && gitSyncConfig ? (
                        <div>
                          <p className="font-medium">{gitSyncConfig.tooltip}</p>
                          {gitSyncStatus && (
                            <p className="text-muted-foreground">
                              {formatGitSyncDetails(gitSyncStatus)}
                            </p>
                          )}
                        </div>
                      ) : (
                        <div>
                          <p className="font-medium">{task.status}</p>
                        </div>
                      )}
                    </TooltipContent>
                  </Tooltip>
                </div>
                <div className="flex flex-col min-w-0 flex-1 gap-0.5">
                  <span className="font-medium text-sm truncate leading-tight">{taskTitle}</span>
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
              {taskTitle.length > 150 ? `${taskTitle.substring(0, 150)}...` : taskTitle}
            </p>
            <p className="text-xs text-muted-foreground">
              {repositoryName} • {getRelativeTime(task.lastMessageAt || task.createdAt)}
            </p>
          </div>
        </TooltipContent>
      </Tooltip>
    </SidebarMenuItem>
  )
}

export function SidebarTasks() {
  const [createTaskOpen, setCreateTaskOpen] = useState(false)

  const {
    data,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList({
    limit: 100,
  })

  const tasks = data?.items

  // Fetch repositories to map IDs to names
  const { data: repositories } = useRepositoriesList()

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
      <TooltipProvider>
        <SidebarGroup>
          <div className="flex items-center justify-between px-2">
            <SidebarGroupLabel className="px-0">Tasks</SidebarGroupLabel>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6"
                  onClick={() => setCreateTaskOpen(true)}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">Create new task</TooltipContent>
            </Tooltip>
          </div>
          <SidebarGroupContent>
            <SidebarTasksSkeleton />
          </SidebarGroupContent>
        </SidebarGroup>
        <CreateTaskDialog open={createTaskOpen} onOpenChange={setCreateTaskOpen} />
      </TooltipProvider>
    )
  }

  if (tasksError) {
    return (
      <TooltipProvider>
        <SidebarGroup>
          <div className="flex items-center justify-between px-2">
            <SidebarGroupLabel className="px-0">Tasks</SidebarGroupLabel>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6"
                  onClick={() => setCreateTaskOpen(true)}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">Create new task</TooltipContent>
            </Tooltip>
          </div>
          <SidebarGroupContent>
            <div className="px-3 py-2 text-sm text-destructive">Failed to load tasks</div>
          </SidebarGroupContent>
        </SidebarGroup>
        <CreateTaskDialog open={createTaskOpen} onOpenChange={setCreateTaskOpen} />
      </TooltipProvider>
    )
  }

  return (
    <TooltipProvider>
      <SidebarGroup>
        <div className="flex items-center justify-between px-2">
          <SidebarGroupLabel className="px-0">Tasks</SidebarGroupLabel>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                onClick={() => setCreateTaskOpen(true)}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="right">Create new task</TooltipContent>
          </Tooltip>
        </div>
        <SidebarGroupContent>
          {tasks && tasks.length > 0 ? (
            <SidebarMenu>
              {tasks.map((task: Task) => {
                const repositoryName = repositoryMap[task.repositoryId] || task.repositoryId
                return (
                  <SidebarTaskItem key={task.taskId} task={task} repositoryName={repositoryName} />
                )
              })}
            </SidebarMenu>
          ) : (
            <div className="px-3 py-2 text-sm text-muted-foreground">No tasks found</div>
          )}
        </SidebarGroupContent>
      </SidebarGroup>
      <CreateTaskDialog open={createTaskOpen} onOpenChange={setCreateTaskOpen} />
    </TooltipProvider>
  )
}
