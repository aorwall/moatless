import type { AgentEvent, Event, Task, TaskIteration } from '@moatless/types'
import { Link } from '@tanstack/react-router'
import { X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTaskEventSubscription } from '@/features/events/contexts/global-event-stream-provider'
import { MinifiedEventItem } from '@/features/tasks/components/minified-event-viewer/minified-event-item'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useCloseTask } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'
import { isTaskRunning } from '@/features/tasks/utils/task-status'
import { type Todo, TodoList } from '@/lib/components/chat/message-history/tool-calls'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader } from '@/lib/components/ui/card'

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

  const agentEvent = event as AgentEvent
  const payload = agentEvent.payload

  // Skip user messages (which are usually tool results)
  if (payload?.type === 'user') {
    return false
  }

  // Include assistant messages (which can be text or tool calls)
  if (payload?.type === 'assistant') {
    return true
  }

  return false
}

function findIterationWithThumbnail(iterations?: TaskIteration[]): TaskIteration | undefined {
  if (!iterations || iterations.length === 0) {
    return undefined
  }
  // Search from last to first to get the most recent thumbnail
  for (let i = iterations.length - 1; i >= 0; i--) {
    if (iterations[i].thumbnailPath) {
      return iterations[i]
    }
  }
  return undefined
}

function extractLatestTodos(events: Event[]): Todo[] | null {
  // Search through all events for TodoWrite tool calls
  let latestTodos: Todo[] | null = null

  for (const event of events) {
    if (event.type !== 'agent' || event.subType !== 'message') {
      continue
    }

    const agentEvent = event as AgentEvent
    const payload = agentEvent.payload

    if (payload?.type !== 'assistant' || !payload.message?.content) {
      continue
    }

    // Handle both string and array content
    const content = payload.message.content
    const contentArray = Array.isArray(content) ? content : []

    // Look for TodoWrite tool calls
    for (const item of contentArray) {
      if (item.type === 'tool_use' && item.tool_name === 'TodoWrite' && item.tool_input) {
        const toolInput = item.tool_input as { todos?: Todo[] }
        if (toolInput.todos && Array.isArray(toolInput.todos)) {
          latestTodos = toolInput.todos
        }
      }
    }
  }

  return latestTodos
}

interface TaskCardProps {
  task: Task
  repositoryName: string
}

export function TaskCard({ task, repositoryName }: TaskCardProps) {
  // Get task title (use name if available, otherwise first iteration's initial message)
  const taskTitle =
    task.name || task.iterations?.[0]?.initialMessage?.split('\n')[0] || task.agentType

  // Get last iteration for result display
  const lastIteration = task.iterations?.[task.iterations.length - 1]

  // Get iteration with thumbnail (may be different from last iteration)
  const thumbnailIteration = findIterationWithThumbnail(task.iterations)

  // Close task mutation
  const closeTaskMutation = useCloseTask(task.taskId)

  // State for live duration updates
  const [runningDuration, setRunningDuration] = useState<string>('')

  // State for real-time events
  const [realtimeEvents, setRealtimeEvents] = useState<Event[]>([])

  // Subscribe to real-time events for this task
  const handleNewEvent = useCallback(
    (event: Event) => {
      if (isTaskRunning(task) && isMessageOrToolEvent(event)) {
        setRealtimeEvents((prev) => [...prev, event])
      }
    },
    [task],
  )

  useTaskEventSubscription(task.taskId, handleNewEvent)

  // Handle close button click
  const handleClose = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      e.stopPropagation()
      closeTaskMutation.mutate()
    },
    [closeTaskMutation],
  )

  const allEvents = useMemo(() => {
    // Remove duplicates based on eventId
    const uniqueEvents = realtimeEvents.filter(
      (event, index, self) => index === self.findIndex((e) => e.eventId === event.eventId),
    )
    return uniqueEvents
  }, [realtimeEvents])

  // Filter events to show only last 3 message/tool events
  const displayedEvents = useMemo(() => {
    return allEvents.filter(isMessageOrToolEvent).slice(-3)
  }, [allEvents])

  // Extract latest todos from all events
  const latestTodos = useMemo(() => {
    const todos = extractLatestTodos(allEvents)
    console.log('TaskCard: latestTodos result', todos)
    return todos
  }, [allEvents])

  // Reset realtime events when task status changes
  useEffect(() => {
    if (!isTaskRunning(task)) {
      setRealtimeEvents([])
    }
  }, [task])

  // Update running duration every second for running tasks
  useEffect(() => {
    if (!(isTaskRunning(task) && lastIteration?.startTime)) {
      setRunningDuration('')
      return
    }

    const updateDuration = () => {
      setRunningDuration(formatRunningDuration(lastIteration.startTime))
    }

    updateDuration() // Initial update
    const interval = setInterval(updateDuration, 1000)

    return () => clearInterval(interval)
  }, [task, lastIteration?.startTime])

  return (
    <Card className="group relative cursor-pointer transition-all hover:-translate-y-0.5 hover:shadow-md">
      {/* Close button */}
      <Button
        variant="ghost"
        size="icon"
        className="absolute right-2 top-2 z-10 h-6 w-6 opacity-0 transition-opacity group-hover:opacity-100"
        onClick={handleClose}
        disabled={closeTaskMutation.isPending}
      >
        <X className="h-4 w-4" />
        <span className="sr-only">Close task</span>
      </Button>

      <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
        <CardHeader className="pb-3">
          <div className="flex items-start gap-2 min-w-0">
            <div className="flex-shrink-0">
              <TaskStatusIcon task={task} showText={false} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-1">
              <h3 className="font-medium text-sm leading-tight line-clamp-2 pr-6">{taskTitle}</h3>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className="truncate">
                  {task.vibe && task.templateId ? task.templateId : repositoryName}
                </span>
                <span className="shrink-0">
                  {isTaskRunning(task) && runningDuration
                    ? runningDuration
                    : getRelativeTime(task.lastMessageAt || task.createdAt)}
                </span>
              </div>
            </div>
          </div>
        </CardHeader>

        <CardContent className="pt-0">
          {isTaskRunning(task) ? (
            // Show TodoList for running tasks with todos, otherwise show events
            latestTodos ? (
              <div className="min-h-[80px] flex items-start">
                <TodoList todos={latestTodos} showSpinner={true} />
              </div>
            ) : displayedEvents.length > 0 ? (
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
            ) : (
              <div className="text-sm text-muted-foreground min-h-[80px] flex items-center">
                Starting task...
              </div>
            )
          ) : (
            // Show result for completed tasks
            <div className={thumbnailIteration?.thumbnailPath ? '' : 'min-h-[80px]'}>
              {latestTodos ? (
                // Show TodoList for completed tasks with todos
                <TodoList todos={latestTodos} showSpinner={false} />
              ) : thumbnailIteration?.thumbnailPath ? (
                // Show thumbnail instead of text result
                <div className="overflow-hidden rounded-md">
                  <img
                    src={`/api/v1/tasks/${task.taskId}/iterations/${thumbnailIteration.iterationNumber}/thumbnail`}
                    alt="Task preview"
                    className="w-full h-auto object-contain"
                    loading="lazy"
                  />
                </div>
              ) : task.status === 'merged' && lastIteration?.gitChanges ? (
                // Special display for merged tasks with git changes
                <div className="space-y-2">
                  <p className="text-sm text-muted-foreground line-clamp-2">
                    {task.description || lastIteration.result || 'Task completed successfully'}
                  </p>
                  <div className="flex items-center gap-3 text-xs">
                    <span className="text-green-600">+{lastIteration.gitChanges.additions}</span>
                    <span className="text-red-600">-{lastIteration.gitChanges.deletions}</span>
                    <span className="text-orange-600">
                      ~{lastIteration.gitChanges.modifications}
                    </span>
                  </div>
                </div>
              ) : (
                // Default: show description if available, otherwise last result
                <p className="text-sm text-muted-foreground line-clamp-4">
                  {task.description || lastIteration?.result || 'No results yet'}
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Link>
    </Card>
  )
}
