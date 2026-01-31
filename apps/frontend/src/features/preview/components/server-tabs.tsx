import { cn } from '@/lib/utils'
import type { ServerEntry } from '../hooks/use-servers'

type WorkspaceView = 'files' | 'preview' | 'diff' | string

interface ServerTabsProps {
  servers: ServerEntry[]
  activeView: WorkspaceView
  onViewChange: (view: WorkspaceView) => void
  className?: string
}

/**
 * Dynamic server tabs component.
 * Renders a tab for each available server plus a "Diff" tab.
 * Server names map to view identifiers:
 * - "preview" server → view "preview"
 * - "files" server → view "files"
 * - other servers → view by server name
 * - Plus a static "Diff" tab → view "diff"
 */
export function ServerTabs({ servers, activeView, onViewChange, className }: ServerTabsProps) {
  // Map server names to view names. The "files" server maps to the "files" view,
  // which on fallback shows the built-in FileExplorer component.
  const serverViews = servers.map((server) => ({
    id: server.name === 'files' ? 'files' : server.name === 'preview' ? 'preview' : server.name,
    label: server.label,
  }))

  // Add the diff tab at the end
  const allTabs = [...serverViews, { id: 'diff', label: 'Diff' }]

  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30 flex-shrink-0', className)}>
      {allTabs.map((tab, index) => (
        <button
          key={tab.id}
          type="button"
          className={cn(
            'px-3 py-1 text-sm font-medium transition-colors',
            index === 0 && 'rounded-l-md',
            index === allTabs.length - 1 && 'rounded-r-md',
            activeView === tab.id
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground',
          )}
          onClick={() => onViewChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  )
}
