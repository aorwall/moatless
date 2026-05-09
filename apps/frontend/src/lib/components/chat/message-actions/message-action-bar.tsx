import type { LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface MessageAction {
  key: string
  icon: LucideIcon
  label: string
  onClick: () => void
  /**
   * Optional badge content, e.g. a feedback count. Rendered next to the icon.
   */
  badge?: React.ReactNode
}

interface MessageActionBarProps {
  actions: MessageAction[]
  className?: string
}

/**
 * Generic action bar shown below an assistant message. Each entry is a
 * button with a Lucide icon. Adding a new action is a one-line append at
 * the call site.
 */
export function MessageActionBar({ actions, className }: MessageActionBarProps) {
  if (actions.length === 0) {
    return null
  }
  return (
    <div
      className={cn('flex items-center gap-1 mt-2 text-muted-foreground', className)}
      data-testid="message-action-bar"
    >
      {actions.map((action) => {
        const Icon = action.icon
        return (
          <button
            type="button"
            key={action.key}
            onClick={action.onClick}
            aria-label={action.label}
            title={action.label}
            className="inline-flex items-center gap-1 rounded p-1 text-xs hover:bg-muted hover:text-foreground transition-colors"
          >
            <Icon className="h-3.5 w-3.5" />
            {action.badge}
          </button>
        )
      })}
    </div>
  )
}
