import type { Event } from '@shared/types/event'
import { AlertCircle, CheckCircle, Clock, Coins } from 'lucide-react'

import { EventHeader } from './event-header'

interface ResultEventProps {
  event: Event
}

interface ResultPayload {
  type: string
  subtype: string
  isError: boolean
  durationMs: number
  durationApiMs: number
  numTurns: number
  result: string
  sessionId: string
  totalCostUsd: number
  usage?: {
    inputTokens?: number
    cacheCreationInputTokens?: number
    cacheReadInputTokens?: number
    outputTokens?: number
  }
}

export function ResultEvent({ event }: ResultEventProps) {
  const payload = event.payload as unknown as ResultPayload

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
          {payload.isError ? (
            <AlertCircle className="h-4 w-4 text-destructive" />
          ) : (
            <CheckCircle className="h-4 w-4 text-green-500" />
          )}
          <span className="text-sm font-medium text-foreground">
            {payload.isError ? 'Failed' : 'Completed'}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
          <div className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            <span>Duration: {formatDuration(payload.durationMs || 0)}</span>
          </div>
          <div className="flex items-center gap-1">
            <span>API Time: {formatDuration(payload.durationApiMs || 0)}</span>
          </div>
          <div className="flex items-center gap-1">
            <span>Turns: {payload.numTurns || 0}</span>
          </div>
          <div className="flex items-center gap-1">
            <Coins className="h-3 w-3" />
            <span>Cost: ${(payload.totalCostUsd || 0).toFixed(4)}</span>
          </div>
        </div>
      </div>
    </EventHeader>
  )
}
