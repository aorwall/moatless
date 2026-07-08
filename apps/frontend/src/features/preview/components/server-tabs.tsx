import type { ServerEntry } from '@moatless/api-client'
import { FileText, type LucideIcon, Monitor, Server, SlidersHorizontal } from 'lucide-react'
import { cn } from '@/lib/utils'

interface ServerTabsProps {
  servers: ServerEntry[]
  activeView: string
  onViewChange: (view: string) => void
  className?: string
}

/** Fixed control view, always available regardless of sandbox/server state. */
export const SANDBOX_VIEW = 'sandbox'

/** Pick an icon for a server tab based on its name. */
function iconForServer(name: string): LucideIcon {
  return name === 'preview' ? Monitor : Server
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

  const isLoading = status === 'starting' || status === 'installing'

  return (
    <span
      className={cn(
        'inline-block h-1.5 w-1.5 rounded-full',
        colorClass,
        isLoading && 'animate-pulse',
      )}
    />
  )
}

/**
 * Dynamic server tabs component.
 * Renders a tab for each available server plus a "Diff" tab.
 * Server names are used directly as view identifiers.
 */
export function ServerTabs({ servers, activeView, onViewChange, className }: ServerTabsProps) {
  const sorted = servers.toSorted((a, b) => (b.default ? 1 : 0) - (a.default ? 1 : 0))
  // Only label the server tabs when there's more than one to disambiguate;
  // Files and Sandbox are always icon-only.
  const showServerLabels = sorted.length > 1
  const allTabs: Array<{
    id: string
    label: string
    status?: ServerEntry['status']
    icon: LucideIcon
    showLabel: boolean
  }> = [
    { id: 'files', label: 'Files', icon: FileText, showLabel: false },
    ...sorted.map((server) => ({
      id: server.name,
      label: server.label,
      status: server.status,
      icon: iconForServer(server.name),
      showLabel: showServerLabels,
    })),
    // Sandbox controls — always last, always available.
    { id: SANDBOX_VIEW, label: 'Sandbox', icon: SlidersHorizontal, showLabel: false },
  ]

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30 flex-shrink-0', className)}>
      {allTabs.map((tab, index) => {
        const Icon = tab.icon
        return (
          <button
            key={tab.id}
            type="button"
            title={tab.label}
            aria-label={tab.label}
            className={cn(
              'px-2.5 py-1 text-sm font-medium transition-colors flex items-center gap-1.5',
              index === 0 && 'rounded-l-md',
              index === allTabs.length - 1 && 'rounded-r-md',
              activeView === tab.id
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => onViewChange(tab.id)}
          >
            {tab.status && <StatusDot status={tab.status} />}
            <Icon className="h-3.5 w-3.5" />
            {tab.showLabel && <span>{tab.label}</span>}
          </button>
        )
      })}
    </div>
  )
}
