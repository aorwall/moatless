import { Clock, DollarSign, InfoIcon, MessageSquare } from 'lucide-react'
import type React from 'react'
import { IsolatedTimer } from '@/lib/components/ui/isolated-timer'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface TaskIterationStatsProps {
  timing: {
    isRunning: boolean
    duration: number
    startTime?: string
  }
  messageCount: number
  cost: number
  willNotResume?: boolean
  onIterationClick: () => void
  onKeyDown: (event: React.KeyboardEvent) => void
}

export function TaskIterationStats({
  timing,
  messageCount,
  cost,
  willNotResume,
  onIterationClick,
  onKeyDown,
}: TaskIterationStatsProps) {
  const formatDuration = (durationMs: number) => {
    const minutes = Math.floor(durationMs / 60000)
    const seconds = Math.floor((durationMs % 60000) / 1000)
    return `${minutes}m ${seconds}s`
  }

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(4)}`
  }

  return (
    <div className="flex justify-center py-2">
      <button
        type="button"
        className="cursor-pointer text-sm text-muted-foreground hover:text-foreground transition-colors flex items-center gap-4 bg-transparent border-none p-0"
        onClick={onIterationClick}
        onKeyDown={onKeyDown}
      >
        <div className="flex items-center gap-1">
          <Clock className="h-4 w-4" />
          {timing.isRunning ? (
            <IsolatedTimer startTime={timing.startTime} />
          ) : (
            <span>{formatDuration(timing.duration)}</span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <MessageSquare className="h-4 w-4" />
          <span>{messageCount} messages</span>
        </div>
        <div className="flex items-center gap-1">
          <DollarSign className="h-4 w-4" />
          <span>{formatCost(cost)}</span>
        </div>
        {willNotResume && (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex items-center gap-1 text-status-warning">
                  <InfoIcon className="h-4 w-4" />
                  <span className="text-xs">Won't resume</span>
                </div>
              </TooltipTrigger>
              <TooltipContent>
                <p>This iteration reached the maximum turn limit.</p>
                <p>Its result will be prepended to the next message.</p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        )}
        <span className="text-xs">• Click to view events</span>
      </button>
    </div>
  )
}
