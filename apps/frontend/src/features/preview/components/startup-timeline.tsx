import { AlertTriangle, CheckCircle2 } from 'lucide-react'
import type { StartupEvent } from '../types'

interface StartupTimelineProps {
  events: StartupEvent[]
}

export function StartupTimeline({ events }: StartupTimelineProps) {
  if (events.length === 0) {
    return null
  }

  return (
    <div className="w-full max-w-sm mt-4">
      <div className="space-y-2">
        {events.map((event, index) => (
          <div key={`${event.reason}-${index}`} className="flex items-start gap-2 text-left">
            {event.eventType === 'Warning' ? (
              <AlertTriangle className="h-4 w-4 text-yellow-500 mt-0.5 shrink-0" />
            ) : (
              <CheckCircle2 className="h-4 w-4 text-green-500 mt-0.5 shrink-0" />
            )}
            <div className="min-w-0">
              <span className="text-xs font-medium text-foreground">{event.reason}</span>
              <p className="text-xs text-muted-foreground truncate">{event.message}</p>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
