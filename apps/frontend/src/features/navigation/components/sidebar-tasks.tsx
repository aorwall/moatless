import type { TaskResponse } from '@moatless/api-client'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { Plus, X } from 'lucide-react'
import type React from 'react'
import { useMemo, useState } from 'react'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { CreateTaskDialog } from '@/features/tasks/components/create-task-dialog'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useCloseTask, useTasksList } from '@/features/tasks/hooks/use-tasks'
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
import type { MessagePreview } from '../hooks/message-preview'
import { type TaskActivity, useTaskActivity } from '../hooks/use-task-activity'
import { useTaskLastMessages } from '../hooks/use-task-last-messages'

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

/** A task row enriched with its live stream activity, used for grouping/sorting. */
interface EnrichedTask {
  task: TaskResponse
  repositoryName: string
  running: boolean
  /** Latest message preview, or `null` when the task has no message yet. */
  preview: string | null
  /**
   * The last few streamed previews (oldest-first) for a running task, shown as
   * several lines of live activity. Empty for idle tasks.
   */
  recentMessages: string[]
  /** Effective last-activity timestamp used for sorting within a group. */
  lastActivityAt: string
}

/** Merge a task with its live activity, deriving running state and preview. */
function enrichTask(
  task: TaskResponse,
  activity: TaskActivity | undefined,
  fetched: MessagePreview | undefined,
): EnrichedTask {
  const repositoryName = task.repositoryName || task.repositoryId
  const running = activity?.running || task.agentState === 'running'
  const lastActivityAt =
    activity?.lastMessageAt || fetched?.at || task.lastMessageAt || task.createdAt

  return {
    task,
    repositoryName,
    running,
    // Prefer the live streamed message, then the message fetched for idle tasks.
    preview: activity?.lastMessage || fetched?.text || null,
    recentMessages: activity?.recentMessages ?? [],
    lastActivityAt,
  }
}

/**
 * Live activity for a running task: the last few streamed previews rendered as
 * separate rows (newest at full strength, older ones dimmed), mirroring the
 * task dashboard so running rows fill a few lines. Falls back to the single
 * latest preview until enough messages have streamed in.
 */
function RunningActivity({
  recentMessages,
  preview,
}: {
  recentMessages: string[]
  preview: string | null
}) {
  const lines = recentMessages.length > 0 ? recentMessages : preview ? [preview] : []
  if (lines.length === 0) {
    return null
  }

  return (
    <div className="flex flex-col gap-0.5">
      {lines.map((line, index) => {
        const isLast = index === lines.length - 1
        return (
          <p
            key={`${index}-${line}`}
            className={`text-xs truncate leading-snug ${
              isLast ? 'text-sidebar-foreground/80' : 'text-sidebar-foreground/45'
            }`}
          >
            {line}
          </p>
        )
      })}
    </div>
  )
}

// Component for rendering individual task item
function SidebarTaskItem({
  item,
  isActive,
  neighborTaskId,
}: {
  item: EnrichedTask
  /** Whether this task is the one currently open in the main view. */
  isActive: boolean
  /** Task to navigate to when closing the active task, or `null` if none. */
  neighborTaskId: string | null
}) {
  const { task, repositoryName, running, preview, recentMessages, lastActivityAt } = item
  const taskTitle = task.name || task.description?.split('\n')[0] || 'Untitled'
  const navigate = useNavigate()
  const closeTask = useCloseTask(task.taskId)

  const handleClose = (event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()

    // If we're closing the task that's currently open, move to the neighbouring
    // task instead of bouncing back to the dashboard.
    if (isActive) {
      if (neighborTaskId) {
        navigate({ to: '/tasks/$task-id', params: { 'task-id': neighborTaskId } })
      } else {
        navigate({ to: '/' })
      }
    }

    closeTask.mutate()
  }

  return (
    <SidebarMenuItem>
      <SidebarMenuButton className="h-auto p-2 hover:bg-sidebar-accent" asChild>
        <Link to="/tasks/$task-id" params={{ 'task-id': task.taskId }}>
          <div className="flex w-full items-start gap-2 min-w-0">
            <div className="flex items-center gap-1 pt-0.5">
              <TaskStatusIcon task={task} showText={false} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-0.5">
              <span className="font-medium text-sm truncate leading-tight pr-5">{taskTitle}</span>
              {running ? (
                <RunningActivity recentMessages={recentMessages} preview={preview} />
              ) : (
                preview && (
                  <p className="text-xs text-sidebar-foreground/70 line-clamp-3 leading-snug">
                    {preview}
                  </p>
                )
              )}
              <div className="flex items-center justify-between gap-2 text-xs text-sidebar-foreground/60">
                <span className="truncate">{repositoryName}</span>
                <span className="shrink-0">{getRelativeTime(lastActivityAt)}</span>
              </div>
            </div>
          </div>
        </Link>
      </SidebarMenuButton>
      <Button
        variant="ghost"
        size="icon"
        aria-label="Close task"
        className="absolute right-1 top-1 z-10 h-5 w-5 opacity-0 transition-opacity group-hover/menu-item:opacity-100 focus-visible:opacity-100"
        onClick={handleClose}
        disabled={closeTask.isPending}
      >
        <X className="h-3.5 w-3.5" />
        <span className="sr-only">Close task</span>
      </Button>
    </SidebarMenuItem>
  )
}

/** Shared header row with the group label and the create-task button. */
function SidebarTasksHeader({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex items-center justify-between px-2">
      <SidebarGroupLabel className="px-0">Tasks</SidebarGroupLabel>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onCreate}>
            <Plus className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="right">Create new task</TooltipContent>
      </Tooltip>
    </div>
  )
}

export function SidebarTasks() {
  const [createTaskOpen, setCreateTaskOpen] = useState(false)
  const { user } = useAuth()
  const activityByTask = useTaskActivity()

  // The task currently open in the main view (if we're on a task route), used to
  // decide where to navigate when its sidebar row is closed.
  const params = useParams({ strict: false })
  const activeTaskId = (params as Record<string, string | undefined>)['task-id'] ?? null

  const {
    data,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList({
    limit: 100,
    status: 'open',
    createdBy: user?.id,
  })

  // Fetch a latest-message preview per task (cached, one request each) so idle
  // tasks show their last message on load; live updates still come from the
  // event stream via `activityByTask` and take precedence.
  const taskIds = useMemo(() => (data?.items ?? []).map((t) => t.taskId), [data?.items])
  const previewByTask = useTaskLastMessages(taskIds)

  // Split tasks into "running" (agent actively working) and the rest, each
  // sorted by most-recent activity. Running state and the live message preview
  // come from the event stream, so this recomputes as those inputs update.
  const { running, open } = useMemo(() => {
    const enriched = (data?.items ?? []).map((task) =>
      enrichTask(task, activityByTask.get(task.taskId), previewByTask.get(task.taskId)),
    )
    const byActivityDesc = (a: EnrichedTask, b: EnrichedTask) =>
      b.lastActivityAt.localeCompare(a.lastActivityAt)

    return {
      running: enriched.filter((t) => t.running).sort(byActivityDesc),
      open: enriched.filter((t) => !t.running).sort(byActivityDesc),
    }
  }, [data?.items, activityByTask, previewByTask])

  // Displayed order (running first, then open) so we can pick the neighbouring
  // task to jump to when the active one is closed.
  const orderedIds = useMemo(
    () => [...running, ...open].map((item) => item.task.taskId),
    [running, open],
  )
  const neighborOf = (taskId: string): string | null => {
    const index = orderedIds.indexOf(taskId)
    if (index === -1) {
      return null
    }
    return orderedIds[index + 1] ?? orderedIds[index - 1] ?? null
  }

  if (isLoadingTasks) {
    return (
      <TooltipProvider>
        <SidebarGroup>
          <SidebarTasksHeader onCreate={() => setCreateTaskOpen(true)} />
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
          <SidebarTasksHeader onCreate={() => setCreateTaskOpen(true)} />
          <SidebarGroupContent>
            <div className="px-3 py-2 text-sm text-destructive">Failed to load tasks</div>
          </SidebarGroupContent>
        </SidebarGroup>
        <CreateTaskDialog open={createTaskOpen} onOpenChange={setCreateTaskOpen} />
      </TooltipProvider>
    )
  }

  const hasTasks = running.length > 0 || open.length > 0

  return (
    <TooltipProvider>
      {running.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel className="flex items-center gap-1.5">
            <span className="relative flex h-1.5 w-1.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-500" />
            </span>
            Running
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {running.map((item) => (
                <SidebarTaskItem
                  key={item.task.taskId}
                  item={item}
                  isActive={item.task.taskId === activeTaskId}
                  neighborTaskId={neighborOf(item.task.taskId)}
                />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      <SidebarGroup>
        <SidebarTasksHeader onCreate={() => setCreateTaskOpen(true)} />
        <SidebarGroupContent>
          {hasTasks ? (
            open.length > 0 ? (
              <SidebarMenu>
                {open.map((item) => (
                  <SidebarTaskItem
                    key={item.task.taskId}
                    item={item}
                    isActive={item.task.taskId === activeTaskId}
                    neighborTaskId={neighborOf(item.task.taskId)}
                  />
                ))}
              </SidebarMenu>
            ) : (
              <div className="px-3 py-2 text-sm text-muted-foreground">All tasks are running</div>
            )
          ) : (
            <div className="px-3 py-2 text-sm text-muted-foreground">No tasks found</div>
          )}
        </SidebarGroupContent>
      </SidebarGroup>
      <CreateTaskDialog open={createTaskOpen} onOpenChange={setCreateTaskOpen} />
    </TooltipProvider>
  )
}
