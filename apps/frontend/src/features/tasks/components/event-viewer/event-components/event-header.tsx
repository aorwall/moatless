import type { Event } from '@moatless/shared/types/event'
import { Clock } from 'lucide-react'
import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Progress } from '@/lib/components/ui/progress'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface EventHeaderProps {
  event: Event
  children: React.ReactNode
}

const formatTimestamp = (timestamp: string) => {
  const date = new Date(timestamp)
  return {
    time: date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }),
    date: date.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
    }),
  }
}

// Map of known usage field names to friendly labels
const usageFieldLabels: Record<string, string> = {
  inputTokens: 'Input',
  outputTokens: 'Output',
  cacheCreationInputTokens: 'Cache Creation',
  cacheReadInputTokens: 'Cache Read',
  totalTokens: 'Total',
}

// Extract numeric usage fields from payload
const extractUsageData = (payload: Record<string, unknown>) => {
  const message = payload.message as Record<string, unknown> | undefined
  const usage = message?.usage
  const usageData: Array<{ label: string; value: number }> = []

  // Only process the usage object if it exists
  if (typeof usage === 'object' && usage !== null) {
    Object.entries(usage).forEach(([key, value]) => {
      if (typeof value === 'number' && value > 0) {
        const label = usageFieldLabels[key] || key.replace(/_/g, ' ')
        usageData.push({ label, value })
      }
    })
  }

  return usageData
}

export function EventHeader({ event, children }: EventHeaderProps) {
  const { time, date } = formatTimestamp(event.timestamp)
  const [showPayloadDialog, setShowPayloadDialog] = useState(false)
  const usageData = extractUsageData(event.payload)

  return (
    <>
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1">{children}</div>

        <div className="flex items-center gap-3 text-xs text-muted-foreground flex-shrink-0">
          {event.contextTokens > 0 && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    className="flex items-center gap-2 cursor-pointer hover:text-foreground transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 rounded"
                    onClick={() => setShowPayloadDialog(true)}
                  >
                    <Progress value={(event.contextTokens / 200000) * 100} className="w-16 h-1.5" />
                    <span className="font-mono text-xs">
                      {event.contextTokens.toLocaleString()}
                    </span>
                  </button>
                </TooltipTrigger>
                <TooltipContent>
                  <div className="space-y-1">
                    <div className="font-semibold">Token Usage</div>
                    {usageData.length > 0 ? (
                      usageData.map(({ label, value }) => (
                        <div key={label} className="flex justify-between gap-3 text-xs">
                          <span className="text-muted-foreground">{label}:</span>
                          <span className="font-mono">{value.toLocaleString()}</span>
                        </div>
                      ))
                    ) : (
                      <div className="text-xs text-muted-foreground">
                        Context: {event.contextTokens.toLocaleString()} tokens
                      </div>
                    )}
                    <div className="pt-1 mt-1 border-t text-xs text-muted-foreground">
                      Click to view full payload
                    </div>
                  </div>
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
          <div className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            <span className="font-mono">{time}</span>
            <span className="hidden sm:inline">{date}</span>
          </div>
        </div>
      </div>

      <Dialog open={showPayloadDialog} onOpenChange={setShowPayloadDialog}>
        <DialogContent className="max-w-3xl max-h-[80vh]">
          <DialogHeader>
            <DialogTitle>Event Payload</DialogTitle>
            <DialogDescription>Full JSON payload for this event</DialogDescription>
          </DialogHeader>
          <div className="overflow-auto">
            <pre className="p-4 bg-muted rounded-lg text-sm">
              <code>{JSON.stringify(event.payload, null, 2)}</code>
            </pre>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
