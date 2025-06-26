import type { SDKResultMessage } from '@shared/types/claude'
import type { ResultEvent as ResultEventType } from '@shared/types/event'
import { AlertCircle, CheckCircle, Clock, Coins } from 'lucide-react'

import { EventHeader } from './event-header'

interface ResultEventProps {
  event: ResultEventType
}

export function ResultEvent({ event }: ResultEventProps) {
  const payload = event.payload as SDKResultMessage

  const formatDuration = (ms: number) => {
    if (ms < 1000) {
      return `${ms}ms`
    }
    const seconds = Math.floor(ms / 1000)
    if (seconds < 60) {
      return `${seconds}s`
    }
    const minutes = Math.floor(seconds / 60)
    const remainingSeconds = seconds % 60
    return `${minutes}m ${remainingSeconds}s`
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          {payload.is_error ? (
            <AlertCircle className="h-4 w-4 text-destructive" />
          ) : (
            <CheckCircle className="h-4 w-4 text-green-500" />
          )}
          <span className="text-sm font-medium text-foreground">
            {payload.is_error ? 'Failed' : 'Completed'}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
          <div className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            <span>Duration: {formatDuration(payload.duration_ms || 0)}</span>
          </div>
          <div className="flex items-center gap-1">
            <span>API Time: {formatDuration(payload.duration_api_ms || 0)}</span>
          </div>
          <div className="flex items-center gap-1">
            <span>Turns: {payload.num_turns || 0}</span>
          </div>
          <div className="flex items-center gap-1">
            <Coins className="h-3 w-3" />
            <span>Cost: ${(payload.total_cost_usd || 0).toFixed(4)}</span>
          </div>
        </div>

        {payload.result && (
          <div className="mt-3 p-3 bg-background rounded border text-sm">
            <div className="text-xs text-muted-foreground mb-2">Result</div>
            <div className="whitespace-pre-wrap">{payload.result}</div>
          </div>
        )}
      </div>
    </EventHeader>
  )
}
