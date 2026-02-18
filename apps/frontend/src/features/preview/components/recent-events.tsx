import { AlertTriangle, CheckCircle2, Server, XCircle } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { SandboxEvent } from '../types'
import { getDisplayMessage } from '../utils/event-messages'

interface RecentEventsProps {
  events: SandboxEvent[]
  className?: string
}

function getEventIcon(event: SandboxEvent) {
  if (event.isError || event.subType === 'failed' || event.subType === 'error') {
    return <XCircle className="h-3 w-3 text-destructive shrink-0" />
  }
  if (
    event.subType === 'installing' ||
    event.subType === 'starting' ||
    event.subType === 'initializing'
  ) {
    return <AlertTriangle className="h-3 w-3 text-yellow-500 shrink-0" />
  }
  if (event.type === 'server') {
    return <Server className="h-3 w-3 text-blue-500 shrink-0" />
  }
  return <CheckCircle2 className="h-3 w-3 text-green-500 shrink-0" />
}

function formatRelativeTime(timestamp: string): string {
  const now = Date.now()
  const then = new Date(timestamp).getTime()
  const diffSeconds = Math.round((now - then) / 1000)

  if (diffSeconds < 5) {
    return 'now'
  }
  if (diffSeconds < 60) {
    return `${diffSeconds}s`
  }
  const diffMinutes = Math.round(diffSeconds / 60)
  if (diffMinutes < 60) {
    return `${diffMinutes}m`
  }
  return new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export function RecentEvents({ events, className }: RecentEventsProps) {
  // Show only the last 3 events
  const recentEvents = events.slice(-3)

  if (recentEvents.length === 0) {
    return null
  }

  return (
    <div className={cn('w-full max-w-sm mt-4', className)}>
      <div className="rounded-md border bg-background/60 backdrop-blur-sm p-2 space-y-1">
        <span className="text-[10px] font-medium uppercase text-muted-foreground tracking-wider px-1">
          Recent events
        </span>
        {recentEvents.map((event) => (
          <div key={event.eventId} className="flex items-center gap-1.5 px-1 py-0.5">
            {getEventIcon(event)}
            <span className="text-xs font-medium truncate flex-1">
              {event.subType}
              <span className="font-normal text-muted-foreground ml-1 truncate">
                {getDisplayMessage(event)}
              </span>
            </span>
            <span className="text-[10px] text-muted-foreground shrink-0 tabular-nums">
              {formatRelativeTime(event.timestamp)}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
