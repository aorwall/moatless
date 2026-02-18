import type { TaskResponse } from '@moatless/api-client'
import type { Event } from '@moatless/types'
import { Link } from '@tanstack/react-router'
import { Globe, Lock, X } from 'lucide-react'
import type React from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useTaskEventSubscription } from '@/features/events/contexts/global-event-stream-provider'
import { MinifiedEventItem } from '@/features/tasks/components/minified-event-viewer/minified-event-item'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useCloseTask, useDeleteTask } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'
import { isTaskRunning } from '@/features/tasks/utils/task-status'
import { type Todo, TodoList } from '@/lib/components/chat/message-history/tool-calls'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader } from '@/lib/components/ui/card'
import { useApiError } from '@/lib/hooks/use-api-error'

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

function isMessageOrToolEvent(event: Event): boolean {
  if (event.type !== 'agent' || event.subType !== 'message') {
    return false
  }

  const payload = event.payload

  // Skip user messages (which are usually tool results)
  if (payload?.type === 'user') {
    return false
  }

  // Include assistant messages (which can be text or tool calls)
  return payload?.type === 'assistant'
}

type ToolUseContent = {
  type: 'tool_use'
  tool_name: string
  tool_input?: unknown
}

interface TodoWriteInput {
  todos?: Todo[]
}

const TODO_STATUSES: Todo['status'][] = ['pending', 'in_progress', 'completed']

function isTodoArray(value: unknown): value is Todo[] {
  return (
    Array.isArray(value) &&
    value.every((todo) => {
      if (typeof todo !== 'object' || todo === null) {
        return false
      }

      const candidate = todo as Partial<Todo>
      return (
        typeof candidate.id === 'string' &&
        typeof candidate.content === 'string' &&
        typeof candidate.status === 'string' &&
        TODO_STATUSES.includes(candidate.status as Todo['status'])
      )
    })
  )
}

function isToolUseContent(value: unknown): value is ToolUseContent {
  if (typeof value !== 'object' || value === null) {
    return false
  }

  const candidate = value as Partial<ToolUseContent>
  return candidate.type === 'tool_use' && typeof candidate.tool_name === 'string'
}

function isTodoWriteInput(value: unknown): value is TodoWriteInput {
  if (typeof value !== 'object' || value === null) {
    return false
  }

  const candidate = value as Partial<TodoWriteInput>
  return !('todos' in candidate) || candidate.todos === undefined || isTodoArray(candidate.todos)
}

function extractLatestTodos(events: Event[]): Todo[] | null {
  // Search through all events for TodoWrite tool calls
  let latestTodos: Todo[] | null = null

  for (const event of events) {
    if (event.type !== 'agent' || event.subType !== 'message') {
      continue
    }

    const payload = event.payload

    if (payload?.type !== 'assistant' || !payload.message?.content) {
      continue
    }

    // Handle both string and array content
    const content = payload.message.content
    const contentArray = Array.isArray(content) ? content : []

    // Look for TodoWrite tool calls
    for (const item of contentArray) {
      if (
        isToolUseContent(item) &&
        item.tool_name === 'TodoWrite' &&
        isTodoWriteInput(item.tool_input)
      ) {
        const todos = item.tool_input?.todos
        if (todos) {
          latestTodos = todos
        }
      }
    }
  }

  return latestTodos
}

interface TaskCardProps {
  task: TaskResponse
  repositoryName: string
}

// Helper: Get unique events by eventId
function getUniqueEvents(events: Event[]): Event[] {
  return events.filter(
    (event, index, self) => index === self.findIndex((e) => e.eventId === event.eventId),
  )
}

// Helper: Get displayed events (last 3 message/tool events)
function getDisplayedEvents(events: Event[]): Event[] {
  return events.filter(isMessageOrToolEvent).slice(-3)
}

// Helper: Setup duration update interval
function setupDurationInterval(
  startTime: Date | string,
  setRunningDuration: (duration: string) => void,
): () => void {
  const updateDuration = () => {
    setRunningDuration(formatRunningDuration(startTime))
  }

  updateDuration() // Initial update
  const interval = setInterval(updateDuration, 1000)

  return () => clearInterval(interval)
}

// Helper: Render running task content
function renderRunningTaskContent(
  latestTodos: Todo[] | null,
  displayedEvents: Event[],
): React.ReactNode {
  if (latestTodos) {
    return (
      <div className="min-h-[80px] flex items-start">
        <TodoList todos={latestTodos} showSpinner={true} />
      </div>
    )
  }

  if (displayedEvents.length > 0) {
    return (
      <div className="min-h-[80px] space-y-px">
        {displayedEvents.map((event, index) => {
          const isLast = index === displayedEvents.length - 1
          const opacity = isLast ? 1 : 0.5 + index * 0.25
          const bgOpacity = isLast ? 0.5 : 0.3

          return (
            <div
              key={event.eventId}
              className="transition-all duration-300 ease-in-out rounded-md px-2 py-1.5 relative"
              style={{
                opacity,
                backgroundColor: `hsl(var(--muted) / ${bgOpacity})`,
              }}
            >
              <MinifiedEventItem event={event} />
            </div>
          )
        })}
      </div>
    )
  }

  return (
    <div className="text-sm text-muted-foreground min-h-[80px] flex items-center">
      Starting task...
    </div>
  )
}

// Helper: Render completed task content
function renderCompletedTaskContent(
  task: TaskResponse,
  latestTodos: Todo[] | null,
): React.ReactNode {
  if (latestTodos) {
    return <TodoList todos={latestTodos} showSpinner={false} />
  }

  // Screenshots are no longer stored at the task level
  // Just show the description
  return (
    <p className="text-sm text-muted-foreground line-clamp-3">
      {task.description || 'No result available'}
    </p>
  )
}

export function TaskCard({ task, repositoryName }: TaskCardProps) {
  // Get task title (use name if available, otherwise first line of description)
  const taskTitle = task.name || task.description?.split('\n')[0] || 'Untitled'

  // Error formatting hook
  const { formatErrorForDisplay } = useApiError()

  const isClosed = task.status === 'closed'

  // Close task mutation with error handling
  const closeTaskMutation = useCloseTask(task.taskId, {
    onError: (error) => {
      try {
        const formattedError = formatErrorForDisplay(error)
        toast.error(formattedError.title, {
          description: formattedError.message,
        })
      } catch {
        // Fallback: Show basic error toast if formatting fails
        toast.error('Failed to close task', {
          description: error.message,
        })
      }
    },
  })

  // Delete task mutation for closed tasks
  const deleteTaskMutation = useDeleteTask({
    onError: (error) => {
      try {
        const formattedError = formatErrorForDisplay(error)
        toast.error(formattedError.title, {
          description: formattedError.message,
        })
      } catch {
        toast.error('Failed to delete task', {
          description: error.message,
        })
      }
    },
  })

  // State for live duration updates
  const [runningDuration, setRunningDuration] = useState<string>('')

  // State for real-time events
  const [realtimeEvents, setRealtimeEvents] = useState<Event[]>([])

  // Subscribe to real-time events for this task
  const taskRunning = isTaskRunning(task)
  const handleNewEvent = useCallback(
    (event: Event) => {
      if (taskRunning && isMessageOrToolEvent(event)) {
        setRealtimeEvents((prev) => [...prev, event])
      }
    },
    [taskRunning],
  )

  useTaskEventSubscription(task.taskId, handleNewEvent)

  // Handle close/delete button click
  const handleClose = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (isClosed) {
        deleteTaskMutation.mutate(task.taskId)
      } else {
        closeTaskMutation.mutate()
      }
    },
    [isClosed, deleteTaskMutation, closeTaskMutation, task.taskId],
  )

  const allEvents = useMemo(() => getUniqueEvents(realtimeEvents), [realtimeEvents])

  // Filter events to show only last 3 message/tool events
  const displayedEvents = useMemo(() => getDisplayedEvents(allEvents), [allEvents])

  // Extract latest todos from all events
  const latestTodos = useMemo(() => extractLatestTodos(allEvents), [allEvents])

  // Reset realtime events when task status changes
  useEffect(() => {
    if (!taskRunning) {
      setRealtimeEvents([])
    }
  }, [taskRunning])

  // Update running duration every second for running tasks
  const taskCreatedAt = task.createdAt
  useEffect(() => {
    if (!(taskRunning && taskCreatedAt)) {
      setRunningDuration('')
      return
    }

    return setupDurationInterval(taskCreatedAt, setRunningDuration)
  }, [taskRunning, taskCreatedAt])

  return (
    <Card className="group relative cursor-pointer transition-all hover:-translate-y-0.5 hover:shadow-md">
      {/* Close button */}
      <Button
        variant="ghost"
        size="icon"
        className="absolute right-2 top-2 z-10 h-6 w-6 opacity-0 transition-opacity group-hover:opacity-100"
        onClick={handleClose}
        disabled={closeTaskMutation.isPending || deleteTaskMutation.isPending}
      >
        <X className="h-4 w-4" />
        <span className="sr-only">{isClosed ? 'Delete task' : 'Close task'}</span>
      </Button>

      <Link to="/tasks/$task-id" params={{ 'task-id': task.taskId }}>
        <CardHeader className="pb-3">
          <div className="flex items-start gap-2 min-w-0">
            <div className="flex-shrink-0">
              <TaskStatusIcon task={task} showText={false} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-1">
              <h3 className="font-medium text-sm leading-tight line-clamp-2 pr-6">{taskTitle}</h3>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <div className="flex items-center gap-1.5 min-w-0">
                  <span className="truncate">{repositoryName}</span>
                  <span title={task.visibility === 'public' ? 'Public' : 'Private'}>
                    {task.visibility === 'public' ? (
                      <Globe className="h-3 w-3 shrink-0" />
                    ) : (
                      <Lock className="h-3 w-3 shrink-0" />
                    )}
                  </span>
                  {(task.createdByDisplayName || task.createdBy) && (
                    <span
                      className="truncate"
                      title={(task.createdByDisplayName || task.createdBy) ?? undefined}
                    >
                      by {task.createdByDisplayName || task.createdBy}
                    </span>
                  )}
                </div>
                <span className="shrink-0">
                  {isTaskRunning(task) && runningDuration
                    ? runningDuration
                    : getRelativeTime(task.lastMessageAt || task.createdAt)}
                </span>
              </div>
              {task.tags && task.tags.length > 0 && (
                <div className="flex gap-1 flex-wrap mt-1">
                  {task.tags.slice(0, 3).map((tag) => (
                    <Badge key={tag.tagId} variant="secondary" className="text-[10px] px-1.5 py-0">
                      {tag.name}
                    </Badge>
                  ))}
                  {task.tags.length > 3 && (
                    <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                      +{task.tags.length - 3}
                    </Badge>
                  )}
                </div>
              )}
            </div>
          </div>
        </CardHeader>

        <CardContent className="pt-0">
          {isTaskRunning(task) ? (
            renderRunningTaskContent(latestTodos, displayedEvents)
          ) : (
            <div className="min-h-[80px]">{renderCompletedTaskContent(task, latestTodos)}</div>
          )}
        </CardContent>
      </Link>
    </Card>
  )
}
