import { ArrowDown } from 'lucide-react'
import { useRef } from 'react'
import type { LifecycleEvent } from '@/features/events/types'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { useAutoScroll } from '@/lib/hooks/use-auto-scroll'
import { cn } from '@/lib/utils'
import {
  describeEvent,
  getDisplayMessage,
  getSourceLabel,
  TONE_TEXT_CLASS,
} from '../utils/event-messages'

interface EventLogProps {
  events: LifecycleEvent[]
}

function EventRow({ event }: { event: LifecycleEvent }) {
  const { label, icon: Icon, tone, chips } = describeEvent(event)
  const message = getDisplayMessage(event)
  const showMessage = message && message.toLowerCase() !== label.toLowerCase()

  return (
    <div className="flex items-start gap-2 text-xs font-mono leading-relaxed">
      <Icon className={cn('h-3.5 w-3.5 shrink-0', TONE_TEXT_CLASS[tone])} />
      <span className="text-muted-foreground shrink-0">
        {new Date(event.timestamp).toLocaleTimeString()}
      </span>
      <span
        className={cn(
          'inline-flex items-center px-1.5 py-0 rounded text-[10px] font-semibold uppercase shrink-0',
          event.type === 'server'
            ? 'bg-blue-500/10 text-blue-500'
            : 'bg-orange-500/10 text-orange-500',
        )}
      >
        {getSourceLabel(event)}
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate">
          <span className={cn('font-semibold', TONE_TEXT_CLASS[tone])}>{label}</span>
          {showMessage && <span className="text-muted-foreground ml-1">{message}</span>}
        </div>
        {chips.length > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-1">
            {chips.map((chip) => (
              <span
                key={chip.label}
                className="inline-flex items-center gap-1 rounded bg-muted px-1 py-0 text-[10px] text-muted-foreground"
              >
                <span className="opacity-60">{chip.label}</span>
                <span className="font-medium text-foreground/80">{chip.value}</span>
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  )
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
      {/* Force the Radix viewport's inner wrapper to `block` (it defaults to
          `display:table`, which sizes to max-content and lets long log lines
          scroll horizontally / defeat the row `truncate`). This keeps the
          console vertical-scroll only and clips overlong lines to width. */}
      <ScrollArea
        ref={scrollAreaRef}
        className="h-full w-full [&>[data-slot=scroll-area-viewport]>div]:!block [&>[data-slot=scroll-area-viewport]>div]:!min-w-0"
      >
        <div className="p-4 space-y-1">
          {events.map((event) => (
            <EventRow key={event.eventId} event={event} />
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
