import type { Task } from '@moatless/types'
import { Link } from '@tanstack/react-router'
import { Plus } from 'lucide-react'
import { useState } from 'react'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { CreateTaskDialog } from '@/features/tasks/components/create-task-dialog'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
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

  if (seconds < 60) {
    return 'just now'
  }
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) {
    return `${minutes}m ago`
  }
  const hours = Math.floor(minutes / 60)
  if (hours < 24) {
    return `${hours}h ago`
  }
  const days = Math.floor(hours / 24)
  if (days < 7) {
    return `${days}d ago`
  }
  const weeks = Math.floor(days / 7)
  if (weeks < 4) {
    return `${weeks}w ago`
  }
  const months = Math.floor(days / 30)
  if (months < 12) {
    return `${months}mo ago`
  }
  const years = Math.floor(days / 365)
  return `${years}y ago`
}

// Component for rendering individual task item
function SidebarTaskItem({ task, repositoryName }: { task: Task; repositoryName: string }) {
  const taskTitle = task.name || task.description?.split('\n')[0] || 'Untitled'

  return (
    <SidebarMenuItem>
      <Tooltip>
        <TooltipTrigger asChild>
          <SidebarMenuButton className="h-auto p-2 hover:bg-sidebar-accent" asChild>
            <Link to="/tasks/$task-id" params={{ 'task-id': task.taskId }}>
              <div className="flex w-full items-center gap-2 min-w-0">
                <div className="flex items-center gap-1">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <div>
                        <TaskStatusIcon task={task} showText={false} />
                      </div>
                    </TooltipTrigger>
                    <TooltipContent side="right" className="text-xs">
                      <p className="font-medium">{task.status}</p>
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
              {task.vibe && task.templateId ? task.templateId : repositoryName} •{' '}
              {getRelativeTime(task.lastMessageAt || task.createdAt)}
            </p>
          </div>
        </TooltipContent>
      </Tooltip>
    </SidebarMenuItem>
  )
}

export function SidebarTasks() {
  const [createTaskOpen, setCreateTaskOpen] = useState(false)
  const { user } = useAuth()

  const {
    data,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList({
    limit: 100,
    status: 'open',
    createdBy: user?.id,
  })

  const tasks = data?.items

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
                return (
                  <SidebarTaskItem
                    key={task.taskId}
                    task={task}
                    repositoryName={task.repositoryName || task.repositoryId}
                  />
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
