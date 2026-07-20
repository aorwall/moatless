import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

interface ListDetailShellProps {
  /** The list pane, typically a `ListDetailSidebar` (or a plain `<aside>`). */
  sidebar: ReactNode
  /** The detail pane, typically a router `<Outlet />`. */
  children: ReactNode
  className?: string
}

/**
 * Two-pane master–detail layout: a fixed-width list sidebar next to a scrollable
 * detail pane. Used by admin section layout routes (e.g. `admin/users.lazy.tsx`).
 */
export function ListDetailShell({ sidebar, children, className }: ListDetailShellProps) {
  return (
    <div className={cn('flex h-full min-h-0 w-full', className)}>
      {sidebar}
      <div className="min-w-0 flex-1 overflow-auto">{children}</div>
    </div>
  )
}
