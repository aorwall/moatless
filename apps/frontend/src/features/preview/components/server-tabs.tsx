import type { ServerEntry } from '@moatless/api-client'
import { cn } from '@/lib/utils'

interface ServerTabsProps {
  servers: ServerEntry[]
  activeView: string
  onViewChange: (view: string) => void
  className?: string
}

/** Small colored dot indicating server status */
function StatusDot({ status }: { status: ServerEntry['status'] }) {
  const colorClass = {
    started: 'bg-green-500',
    starting: 'bg-amber-500',
    installing: 'bg-amber-500',
    failed: 'bg-red-500',
    stopped: 'bg-muted-foreground/40',
  }[status]

  return <span className={cn('inline-block h-1.5 w-1.5 rounded-full', colorClass)} />
}

/**
 * Dynamic server tabs component.
 * Renders a tab for each available server plus a "Diff" tab.
 * Server names are used directly as view identifiers.
 */
export function ServerTabs({ servers, activeView, onViewChange, className }: ServerTabsProps) {
  const sorted = servers.toSorted((a, b) => (b.default ? 1 : 0) - (a.default ? 1 : 0))
  const serverViews = sorted.map((server) => ({
    id: server.name,
    label: server.label,
    status: server.status,
  }))

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30 flex-shrink-0', className)}>
      {serverViews.map((tab, index) => (
        <button
          key={tab.id}
          type="button"
          className={cn(
            'px-3 py-1 text-sm font-medium transition-colors flex items-center gap-1.5',
            index === 0 && 'rounded-l-md',
            index === serverViews.length - 1 && 'rounded-r-md',
            activeView === tab.id
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground',
          )}
          onClick={() => onViewChange(tab.id)}
        >
          {tab.status && <StatusDot status={tab.status} />}
          {tab.label}
        </button>
      ))}
    </div>
  )
}
