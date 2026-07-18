import type { ReactNode } from 'react'

interface DetailHeaderProps {
  /** Leading visual, e.g. a `ListAvatar size="lg"` or an icon. */
  avatar?: ReactNode
  title: ReactNode
  subtitle?: ReactNode
  /** Badges rendered next to the title. */
  badges?: ReactNode
  /** Actions rendered on the right (e.g. Edit / Delete buttons). */
  actions?: ReactNode
}

/**
 * Header for a detail/edit pane: optional avatar, title + badges, subtitle, and
 * a right-aligned actions slot.
 */
export function DetailHeader({ avatar, title, subtitle, badges, actions }: DetailHeaderProps) {
  return (
    <div className="flex items-start gap-4">
      {avatar}
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <h1 className="truncate text-2xl font-semibold">{title}</h1>
          {badges}
        </div>
        {subtitle !== undefined ? (
          <p className="text-sm text-muted-foreground">{subtitle}</p>
        ) : null}
      </div>
      {actions !== undefined ? <div className="shrink-0">{actions}</div> : null}
    </div>
  )
}
