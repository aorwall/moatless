import { type MouseEvent, useMemo, useState } from 'react'
import type { TaskMessageTokenUsage } from '@/features/tasks/api/tasks'
import { cn } from '@/lib/utils'

interface MessageTokenUsageChartProps {
  messages: TaskMessageTokenUsage[]
}

const numberFormat = new Intl.NumberFormat()

// Fixed coordinate space; each SVG scales to its container via preserveAspectRatio.
const VIEW_W = 100
const VIEW_H = 100
// Vertical padding keeps the top/bottom of the line off the clipping edges.
const PAD_Y = 8

// Input/output and cached-read tokens differ by ~1000x, so they can't share one
// linear axis. Render each as its own small-multiple line with its own max.
const SERIES: {
  key: 'inputTokens' | 'outputTokens' | 'cachedInputTokens'
  title: string
  colorClass: string
}[] = [
  { key: 'inputTokens', title: 'Input', colorClass: 'text-chart-1' },
  { key: 'outputTokens', title: 'Output', colorClass: 'text-chart-2' },
  { key: 'cachedInputTokens', title: 'Cached read', colorClass: 'text-chart-3' },
]

export function MessageTokenUsageChart({ messages }: MessageTokenUsageChartProps) {
  // Label by position, not turn: a turn spans every message of one agent run,
  // and queued messages have no turn until the run that consumes them acks —
  // so turn numbers are neither unique nor always present along this axis.
  const labels = useMemo(() => messages.map((_message, index) => index + 1), [messages])

  if (messages.length === 0) {
    return null
  }

  const firstLabel = labels[0]
  const lastLabel = labels.at(-1)

  return (
    <div className="space-y-3">
      {SERIES.map((series) => (
        <MiniLineChart
          key={series.key}
          title={series.title}
          colorClass={series.colorClass}
          labels={labels}
          values={messages.map((message) => message[series.key])}
        />
      ))}
      <div className="flex items-center justify-between text-[10px] text-muted-foreground tabular-nums">
        <span>#{firstLabel}</span>
        <span>Message index ({messages.length})</span>
        <span>#{lastLabel}</span>
      </div>
    </div>
  )
}

interface MiniLineChartProps {
  title: string
  colorClass: string
  labels: number[]
  values: number[]
}

interface ChartPoint {
  label: number
  value: number
  /** 0-100 horizontal position. */
  x: number
  /** 0-100 vertical position (SVG space, 0 = top). */
  y: number
}

function MiniLineChart({ title, colorClass, labels, values }: MiniLineChartProps) {
  const [activeIndex, setActiveIndex] = useState<number | null>(null)

  const max = useMemo(() => Math.max(0, ...values), [values])

  const points = useMemo<ChartPoint[]>(() => {
    return values.map((value, index) => {
      const fraction = max > 0 ? value / max : 0
      const x = values.length > 1 ? (index / (values.length - 1)) * VIEW_W : VIEW_W / 2
      const y = PAD_Y + (1 - fraction) * (VIEW_H - PAD_Y * 2)
      return { label: labels[index] ?? index + 1, value, x, y }
    })
  }, [values, labels, max])

  const first = points[0]
  const last = points.at(-1)
  if (!(first && last)) {
    return null
  }

  const linePath = points
    .map((point, index) => `${index === 0 ? 'M' : 'L'}${point.x},${point.y}`)
    .join(' ')
  const areaPath = `${linePath} L${last.x},${VIEW_H} L${first.x},${VIEW_H} Z`

  const active = activeIndex !== null ? points[activeIndex] : null

  // Map the pointer's horizontal position to the nearest data point. A single
  // overlay keeps hover O(1) in DOM nodes even with thousands of messages.
  const handleMove = (event: MouseEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    if (rect.width <= 0 || points.length === 0) {
      return
    }
    const fraction = (event.clientX - rect.left) / rect.width
    const index = points.length > 1 ? Math.round(fraction * (points.length - 1)) : 0
    setActiveIndex(Math.min(Math.max(index, 0), points.length - 1))
  }

  return (
    <div className={cn('space-y-1', colorClass)}>
      <div className="flex items-center justify-between text-[10px] tabular-nums">
        <span className="flex items-center gap-1 font-medium text-foreground">
          <span className="h-1.5 w-1.5 rounded-full bg-current" />
          {title}
        </span>
        <span className="text-muted-foreground">max {numberFormat.format(max)}</span>
      </div>
      <div className="relative h-16 w-full">
        <svg
          viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
          preserveAspectRatio="none"
          className="h-full w-full overflow-visible"
          role="img"
          aria-label={`Line graph of ${title.toLowerCase()} tokens by message index`}
        >
          <line
            x1={0}
            y1={VIEW_H}
            x2={VIEW_W}
            y2={VIEW_H}
            className="stroke-border"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
          <path d={areaPath} className="fill-current opacity-10" />
          <path
            d={linePath}
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            strokeLinejoin="round"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>

        {/* Single HTML overlay drives hover (crosshair + tooltip), positioned in %. */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer-only hover enhancement; the chart itself is labeled via the SVG role="img". */}
        <div
          className="absolute inset-0"
          onMouseMove={handleMove}
          onMouseLeave={() => setActiveIndex(null)}
        >
          {active && (
            <>
              <div
                className="pointer-events-none absolute inset-y-0 w-px -translate-x-1/2 bg-current opacity-30"
                style={{ left: `${active.x}%` }}
              />
              <div
                className="pointer-events-none absolute z-10 h-2 w-2 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-background bg-current"
                style={{ left: `${active.x}%`, top: `${active.y}%` }}
              />
              <div
                className="pointer-events-none absolute z-20 -translate-y-full -translate-x-1/2 whitespace-nowrap rounded border bg-popover px-1.5 py-1 text-[10px] leading-tight text-popover-foreground shadow-sm"
                style={{ left: `${Math.min(Math.max(active.x, 12), 88)}%`, top: `${active.y}%` }}
              >
                <span className="text-muted-foreground">#{active.label}</span>{' '}
                <span className="font-medium tabular-nums">
                  {numberFormat.format(active.value)}
                </span>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
