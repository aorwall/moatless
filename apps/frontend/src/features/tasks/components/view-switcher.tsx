import type { ServerEntry } from '@moatless/api-client'
import {
  ChevronDown,
  FileText,
  Globe,
  type LucideIcon,
  MessageSquare,
  Server,
  SlidersHorizontal,
} from 'lucide-react'
import { useState } from 'react'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { cn } from '@/lib/utils'
import { SANDBOX_VIEW } from '../types/view-registry'

interface ViewSwitcherProps {
  currentView: string
  onViewChange: (view: string) => void
  servers?: ServerEntry[] | undefined
  className?: string | undefined
}

/** Pick an icon based on server name */
function iconForServer(name: string): LucideIcon {
  switch (name) {
    case 'files':
      return FileText
    case 'preview':
      return Globe
    default:
      return Server
  }
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

/** Shared classes for a single segmented-control button. */
function tabClasses(isActive: boolean, isFirst: boolean, isLast: boolean) {
  return cn(
    'px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-1.5',
    isFirst && 'rounded-l-md',
    isLast && 'rounded-r-md',
    isActive
      ? 'bg-background text-foreground shadow-sm'
      : 'text-muted-foreground hover:text-foreground',
  )
}

/**
 * Consolidated preview-server picker for mobile. Multiple servers collapse into
 * a single globe dropdown so the switcher never grows an icon per server; the
 * trigger reflects the active (or default) server's status, and the menu lists
 * every server.
 */
function ServerDropdown({
  servers,
  currentView,
  onViewChange,
}: {
  servers: ServerEntry[]
  currentView: string
  onViewChange: (view: string) => void
}) {
  const [open, setOpen] = useState(false)
  const isActive = servers.some((s) => s.name === currentView)
  const displayed =
    servers.find((s) => s.name === currentView) ?? servers.find((s) => s.default) ?? servers[0]

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={tabClasses(isActive, false, false)}
          aria-label="Select preview server"
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
                  server.name === currentView
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

export function ViewSwitcher({ currentView, onViewChange, servers, className }: ViewSwitcherProps) {
  const hasServers = servers && servers.length > 0
  const hasMultipleServers = servers ? servers.length > 1 : false

  // Fixed tabs before/after the server slot. Files is always offered on mobile —
  // the server list never contains a `files` entry, so it's added explicitly.
  const leadingTabs: Array<{ value: string; label: string; icon: LucideIcon }> = [
    { value: 'chat', label: 'Chat', icon: MessageSquare },
    { value: 'files', label: 'Files', icon: FileText },
  ]

  // Server slot: a single tab, the consolidated dropdown, or the preview
  // fallback used before the server list has loaded.
  const singleServer = hasServers && !hasMultipleServers ? servers[0] : undefined

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30', className)}>
      {leadingTabs.map((tab) => {
        const Icon = tab.icon
        const isActive = currentView === tab.value
        return (
          <button
            key={tab.value}
            type="button"
            className={tabClasses(isActive, tab.value === 'chat', false)}
            onClick={() => onViewChange(tab.value)}
            aria-label={tab.label}
            aria-pressed={isActive}
          >
            <Icon className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{tab.label}</span>
          </button>
        )
      })}

      {hasMultipleServers && servers ? (
        <ServerDropdown servers={servers} currentView={currentView} onViewChange={onViewChange} />
      ) : singleServer ? (
        <button
          type="button"
          className={tabClasses(currentView === singleServer.name, false, false)}
          onClick={() => onViewChange(singleServer.name)}
          aria-label={singleServer.label}
          aria-pressed={currentView === singleServer.name}
        >
          <StatusDot status={singleServer.status} />
          {(() => {
            const Icon = iconForServer(singleServer.name)
            return <Icon className="h-3.5 w-3.5" />
          })()}
          <span className="hidden sm:inline">{singleServer.label}</span>
        </button>
      ) : (
        // Fallback before servers load.
        <button
          type="button"
          className={tabClasses(currentView === 'preview', false, false)}
          onClick={() => onViewChange('preview')}
          aria-label="Preview"
          aria-pressed={currentView === 'preview'}
        >
          <Globe className="h-3.5 w-3.5" />
          <span className="hidden sm:inline">Preview</span>
        </button>
      )}

      <button
        type="button"
        className={tabClasses(currentView === SANDBOX_VIEW, false, true)}
        onClick={() => onViewChange(SANDBOX_VIEW)}
        aria-label="Sandbox"
        aria-pressed={currentView === SANDBOX_VIEW}
      >
        <SlidersHorizontal className="h-3.5 w-3.5" />
        <span className="hidden sm:inline">Sandbox</span>
      </button>
    </div>
  )
}
