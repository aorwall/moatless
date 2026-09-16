import type { ServerEntry } from '@moatless/api-client'
import {
  ChevronDown,
  FileText,
  Globe,
  type LucideIcon,
  Server,
  SlidersHorizontal,
} from 'lucide-react'
import { useState } from 'react'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { cn } from '@/lib/utils'

interface ViewTabsProps {
  servers: ServerEntry[]
  activeView: string
  onViewChange: (view: string) => void
  className?: string
}

/** Fixed control view, always available regardless of sandbox/server state. */
export const SANDBOX_VIEW = 'sandbox'

/** Pick an icon for a server tab based on its name. */
function iconForServer(name: string): LucideIcon {
  return name === 'preview' ? Globe : Server
}

/** Small colored dot indicating server status */
function StatusDot({ status }: { status: ServerEntry['status'] }) {
  const colorClass = {
    started: 'bg-green-500',
    starting: 'bg-amber-500',
    installing: 'bg-amber-500',
    failed: 'bg-red-500',
    stopped: 'bg-muted-foreground/40',
    disabled: 'bg-muted-foreground/40',
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

/** Shared classes for a single segmented-control button. */
function tabClasses(isActive: boolean, isFirst: boolean, isLast: boolean) {
  return cn(
    'px-2.5 py-1 text-sm font-medium transition-colors flex items-center gap-1.5',
    isFirst && 'rounded-l-md',
    isLast && 'rounded-r-md',
    isActive
      ? 'bg-background text-foreground shadow-sm'
      : 'text-muted-foreground hover:text-foreground',
  )
}

/**
 * Consolidated preview-server picker. When a repository exposes more than one
 * server we collapse the individual server tabs into a single globe dropdown so
 * the header doesn't grow an icon per server. The trigger reflects the active
 * (or default) server's status and label; the menu lists every server.
 */
function ServerDropdown({
  servers,
  activeView,
  onViewChange,
  isFirst,
  isLast,
}: {
  servers: ServerEntry[]
  activeView: string
  onViewChange: (view: string) => void
  isFirst: boolean
  isLast: boolean
}) {
  const [open, setOpen] = useState(false)
  const isActive = servers.some((s) => s.name === activeView)
  const displayed =
    servers.find((s) => s.name === activeView) ?? servers.find((s) => s.default) ?? servers[0]

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title="Select preview server"
          aria-label="Select preview server"
          className={tabClasses(isActive, isFirst, isLast)}
        >
          {displayed && <StatusDot status={displayed.status} />}
          <Globe className="h-3.5 w-3.5" />
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56 p-1">
        <div className="flex flex-col">
          {servers.map((server) => {
            const Icon = iconForServer(server.name)
            return (
              <button
                key={server.name}
                type="button"
                className={cn(
                  'flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm text-left transition-colors',
                  server.name === activeView
                    ? 'bg-accent text-accent-foreground'
                    : 'hover:bg-accent hover:text-accent-foreground',
                )}
                onClick={() => {
                  onViewChange(server.name)
                  setOpen(false)
                }}
              >
                <StatusDot status={server.status} />
                <Icon className="h-3.5 w-3.5 flex-shrink-0" />
                <span className="truncate">{server.label}</span>
              </button>
            )
          })}
        </div>
      </PopoverContent>
    </Popover>
  )
}

/**
 * Dynamic workspace view tabs.
 * Renders a tab for Files, the available server(s), and the Sandbox controls.
 * A single server renders as one globe tab; multiple servers collapse into a
 * globe dropdown. Server names are used directly as view identifiers.
 */
export function ViewTabs({ servers, activeView, onViewChange, className }: ViewTabsProps) {
  const sorted = servers.toSorted((a, b) => (b.default ? 1 : 0) - (a.default ? 1 : 0))
  const hasMultipleServers = sorted.length > 1

  // Files is always first; Sandbox is always last. The server slot in between is
  // either a single tab or the consolidated dropdown.
  const filesActive = activeView === 'files'
  const sandboxActive = activeView === SANDBOX_VIEW

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30 flex-shrink-0', className)}>
      <button
        type="button"
        title="Files"
        aria-label="Files"
        className={tabClasses(filesActive, true, false)}
        onClick={() => onViewChange('files')}
      >
        <FileText className="h-3.5 w-3.5" />
      </button>

      {hasMultipleServers ? (
        <ServerDropdown
          servers={sorted}
          activeView={activeView}
          onViewChange={onViewChange}
          isFirst={false}
          isLast={false}
        />
      ) : (
        sorted.map((server) => {
          const Icon = iconForServer(server.name)
          const isActive = activeView === server.name
          return (
            <button
              key={server.name}
              type="button"
              title={server.label}
              aria-label={server.label}
              className={tabClasses(isActive, false, false)}
              onClick={() => onViewChange(server.name)}
            >
              <StatusDot status={server.status} />
              <Icon className="h-3.5 w-3.5" />
            </button>
          )
        })
      )}

      <button
        type="button"
        title="Sandbox"
        aria-label="Sandbox"
        className={tabClasses(sandboxActive, false, true)}
        onClick={() => onViewChange(SANDBOX_VIEW)}
      >
        <SlidersHorizontal className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}
