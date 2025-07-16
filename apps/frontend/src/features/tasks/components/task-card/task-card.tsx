import type { Event, MessageEvent, Task } from '@moatless/types'
import { Link } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTaskEventSubscription } from '@/features/events/contexts/global-event-stream-provider'
import { GitStatusIcon } from '@/features/tasks/components/git-status-icon'
import { MinifiedEventItem } from '@/features/tasks/components/minified-event-viewer/minified-event-item'
import { useTaskEvents } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'
import { Card, CardContent, CardHeader } from '@/lib/components/ui/card'

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

function isMessageOrToolEvent(event: Event | MergedEvent): boolean {
  if (event.type !== 'message') return false

  const messageEvent = event as MessageEvent
  const payload = messageEvent.payload

  // Skip user messages (which are usually tool results)
  if (payload?.type === 'user') return false

  // Include assistant messages (which can be text or tool calls)
  if (payload?.type === 'assistant') return true

  return false
}

interface TaskCardProps {
  task: Task
  repositoryName: string
}

export function TaskCard({ task, repositoryName }: TaskCardProps) {
  // Get task title from first iteration's initial message
  const taskTitle = task.iterations?.[0]?.initialMessage?.split('\n')[0] || task.agentType

  // Get last iteration for result display
  const lastIteration = task.iterations?.[task.iterations.length - 1]

  // Get initial events for running tasks
  const { data: eventsData } = useTaskEvents(task.taskId, undefined, {
    enabled: task.status === 'running',
    refetchInterval: false, // Real-time updates handled by global event stream
  })

  // State for live duration updates
  const [runningDuration, setRunningDuration] = useState<string>('')

  // State for real-time events
  const [realtimeEvents, setRealtimeEvents] = useState<Event[]>([])

  // Subscribe to real-time events for this task
  const handleNewEvent = useCallback(
    (event: Event) => {
      if (task.status === 'running' && isMessageOrToolEvent(event)) {
        setRealtimeEvents((prev) => [...prev, event])
      }
    },
    [task.status],
  )

  useTaskEventSubscription(task.taskId, handleNewEvent)

  // Combine initial events with real-time events
  const allEvents = useMemo(() => {
    const events = [...(eventsData || []), ...realtimeEvents]
    // Remove duplicates based on eventId
    const uniqueEvents = events.filter(
      (event, index, self) => index === self.findIndex((e) => e.eventId === event.eventId),
    )
    return uniqueEvents
  }, [eventsData, realtimeEvents])

  // Filter events to show only last 3 message/tool events
  const displayedEvents = useMemo(() => {
    return allEvents.filter(isMessageOrToolEvent).slice(-3)
  }, [allEvents])

  // Reset realtime events when task status changes
  useEffect(() => {
    if (task.status !== 'running') {
      setRealtimeEvents([])
    }
  }, [task.status])

  // Update running duration every second for running tasks
  useEffect(() => {
    if (task.status !== 'running' || !lastIteration?.startTime) {
      setRunningDuration('')
      return
    }

    const updateDuration = () => {
      setRunningDuration(formatRunningDuration(lastIteration.startTime))
    }

    updateDuration() // Initial update
    const interval = setInterval(updateDuration, 1000)

    return () => clearInterval(interval)
  }, [task.status, lastIteration?.startTime])

  return (
    <Card className="cursor-pointer transition-all hover:-translate-y-0.5 hover:shadow-md">
      <Link to="/tasks/$taskId" params={{ taskId: task.taskId }}>
        <CardHeader className="pb-3">
          <div className="flex items-start gap-2 min-w-0">
            <div className="flex-shrink-0">
              <GitStatusIcon gitStatus={task.iterations?.[task.iterations.length - 1]?.gitStatus} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-1">
              <h3 className="font-medium text-sm leading-tight line-clamp-2">{taskTitle}</h3>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span className="truncate">{repositoryName}</span>
                <span className="shrink-0">
                  {task.status === 'running' && runningDuration
                    ? runningDuration
                    : getRelativeTime(task.lastMessageAt || task.createdAt)}
                </span>
              </div>
            </div>
          </div>
        </CardHeader>

        <CardContent className="pt-0">
          {task.status === 'running' ? (
            // Show events for running tasks
            displayedEvents.length > 0 ? (
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
            <div className="min-h-[80px]">
              {task.status === 'merged' && lastIteration?.gitChanges ? (
                // Special display for merged tasks with git changes
                <div className="space-y-2">
                  <p className="text-sm text-muted-foreground line-clamp-2">
                    {lastIteration.result || 'Task completed successfully'}
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
                // Default: show last result
                <p className="text-sm text-muted-foreground line-clamp-4">
                  {lastIteration?.result || lastIteration?.error?.message || 'No results yet'}
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Link>
    </Card>
  )
}
