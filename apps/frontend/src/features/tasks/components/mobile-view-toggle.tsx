import type { ServerEntry } from '@moatless/api-client'
import { FileText, MessageSquare, Monitor, Server, SlidersHorizontal } from 'lucide-react'
import { SANDBOX_VIEW } from '@/features/preview/components/server-tabs'
import { cn } from '@/lib/utils'

interface MobileViewToggleProps {
  currentView: string
  onViewChange: (view: string) => void
  servers?: ServerEntry[]
  className?: string
}

/** Pick an icon based on server name */
function iconForServer(name: string) {
  switch (name) {
    case 'files':
      return FileText
    case 'preview':
      return Monitor
    default:
      return Server
  }
}

export function MobileViewToggle({
  currentView,
  onViewChange,
  servers,
  className,
}: MobileViewToggleProps) {
  // Build tabs dynamically: chat + each server
  const tabs: Array<{ value: string; label: string; icon: typeof MessageSquare }> = [
    { value: 'chat', label: 'Chat', icon: MessageSquare },
  ]

  if (servers && servers.length > 0) {
    for (const server of servers) {
      tabs.push({
        value: server.name,
        label: server.label,
        icon: iconForServer(server.name),
      })
    }
  } else {
    // Fallback when servers aren't loaded yet
    tabs.push({ value: 'files', label: 'Files', icon: FileText })
    tabs.push({ value: 'preview', label: 'Preview', icon: Monitor })
  }

  // Sandbox controls — always available, regardless of server state.
  tabs.push({ value: SANDBOX_VIEW, label: 'Sandbox', icon: SlidersHorizontal })

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30', className)}>
      {tabs.map((tab, index) => {
        const Icon = tab.icon
        const isActive = currentView === tab.value
        const isFirst = index === 0
        const isLast = index === tabs.length - 1

        return (
          <button
            key={tab.value}
            type="button"
            className={cn(
              'px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-1.5',
              isFirst && 'rounded-l-md',
              isLast && 'rounded-r-md',
              isActive
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => onViewChange(tab.value)}
            aria-label={tab.label}
            aria-pressed={isActive}
          >
            <Icon className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{tab.label}</span>
          </button>
        )
      })}
    </div>
  )
}
