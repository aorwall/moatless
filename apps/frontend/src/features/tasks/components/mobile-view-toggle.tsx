import { FileText, MessageSquare, Monitor } from 'lucide-react'
import type { MobileView } from '@/features/tasks/contexts/task-ui-provider'
import { cn } from '@/lib/utils'

interface MobileViewToggleProps {
  currentView: MobileView
  onViewChange: (view: MobileView) => void
  className?: string
}

const views = [
  { value: 'chat' as const, label: 'Chat', icon: MessageSquare },
  { value: 'files' as const, label: 'Files', icon: FileText },
  { value: 'preview' as const, label: 'Preview', icon: Monitor },
] as const

export function MobileViewToggle({ currentView, onViewChange, className }: MobileViewToggleProps) {
  return (
    <div className={cn('flex items-center border rounded-md bg-muted/30', className)}>
      {views.map((view, index) => {
        const Icon = view.icon
        const isActive = currentView === view.value
        const isFirst = index === 0
        const isLast = index === views.length - 1

        return (
          <button
            key={view.value}
            type="button"
            className={cn(
              'px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-1.5',
              isFirst && 'rounded-l-md',
              isLast && 'rounded-r-md',
              isActive
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
            onClick={() => onViewChange(view.value)}
            aria-label={view.label}
            aria-pressed={isActive}
          >
            <Icon className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{view.label}</span>
          </button>
        )
      })}
    </div>
  )
}
