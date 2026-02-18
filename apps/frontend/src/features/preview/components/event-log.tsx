import { AlertTriangle, ArrowDown, CheckCircle2, Server, XCircle } from 'lucide-react'
import { useRef } from 'react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { useAutoScroll } from '@/lib/hooks/use-auto-scroll'
import { cn } from '@/lib/utils'
import type { SandboxEvent } from '../types'
import { getDisplayMessage, getSourceLabel } from '../utils/event-messages'

interface EventLogProps {
  events: SandboxEvent[]
}

function getEventIcon(event: SandboxEvent) {
  if (event.isError) {
    return <XCircle className="h-3.5 w-3.5 text-destructive shrink-0" />
  }
  if (event.subType === 'failed' || event.subType === 'error') {
    return <XCircle className="h-3.5 w-3.5 text-destructive shrink-0" />
  }
  if (
    event.subType === 'installing' ||
    event.subType === 'starting' ||
    event.subType === 'initializing'
  ) {
    return <AlertTriangle className="h-3.5 w-3.5 text-yellow-500 shrink-0" />
  }
  if (event.type === 'server') {
    return <Server className="h-3.5 w-3.5 text-blue-500 shrink-0" />
  }
  return <CheckCircle2 className="h-3.5 w-3.5 text-green-500 shrink-0" />
}

function getEventColor(event: SandboxEvent): string {
  if (event.isError || event.subType === 'failed' || event.subType === 'error') {
    return 'text-destructive'
  }
  if (
    event.subType === 'installing' ||
    event.subType === 'starting' ||
    event.subType === 'initializing'
  ) {
    return 'text-yellow-500'
  }
  return 'text-foreground'
}

export function EventLog({ events }: EventLogProps) {
  const bottomRef = useRef<HTMLDivElement>(null)

  const {
    scrollRef: scrollAreaRef,
    isAtBottom,
    scrollToBottom,
  } = useAutoScroll({
    scrollOnMount: true,
    dependencies: [events],
  })

  if (events.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">No events yet</p>
          <p className="mt-2 text-sm">Sandbox and server lifecycle events will appear here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-background relative">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full">
        <div className="p-4 space-y-1">
          {events.map((event) => (
            <div
              key={event.eventId}
              className="flex items-start gap-2 text-xs font-mono leading-relaxed"
            >
              {getEventIcon(event)}
              <span className="text-muted-foreground shrink-0">
                {new Date(event.timestamp).toLocaleTimeString()}
              </span>
              <span
                className={cn(
                  'inline-flex items-center px-1.5 py-0 rounded text-[10px] font-semibold uppercase shrink-0',
                  event.type === 'sandbox'
                    ? 'bg-orange-500/10 text-orange-500'
                    : 'bg-blue-500/10 text-blue-500',
                )}
              >
                {getSourceLabel(event)}
              </span>
              <span className={cn('truncate', getEventColor(event))}>
                <span className="font-semibold">{event.subType}</span>
                <span className="text-muted-foreground ml-1">{getDisplayMessage(event)}</span>
              </span>
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
      </ScrollArea>

      {!isAtBottom && (
        <div className="absolute bottom-4 right-4 z-10">
          <Button
            variant="outline"
            size="sm"
            onClick={scrollToBottom}
            className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
          >
            <ArrowDown className="h-4 w-4" />
            Scroll to bottom
          </Button>
        </div>
      )}
    </div>
  )
}
