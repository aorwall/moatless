import { Link, type LinkProps } from '@tanstack/react-router'
import { Loader2, Search } from 'lucide-react'
import type { ComponentType, ReactNode, SVGProps } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error'

interface ListDetailSidebarProps {
  icon: ComponentType<SVGProps<SVGSVGElement>>
  title: string
  /** Optional count shown as a badge next to the title. */
  count?: number | undefined
  /** Optional action rendered on the right of the header (e.g. a "+ New" link). */
  action?: ReactNode
  search: string
  onSearchChange: (value: string) => void
  searchPlaceholder?: string | undefined
  isLoading: boolean
  error?: unknown
  /** Whether the (filtered) list is empty — controls the empty message. */
  isEmpty: boolean
  emptyMessage?: string | undefined
  /** Tailwind width class for the sidebar. Defaults to `w-72`. */
  width?: string | undefined
  /** `ListDetailSidebar.Item` rows. */
  children: ReactNode
}

function ListDetailSidebarRoot({
  icon: Icon,
  title,
  count,
  action,
  search,
  onSearchChange,
  searchPlaceholder = 'Search...',
  isLoading,
  error,
  isEmpty,
  emptyMessage = 'No items found',
  width = 'w-72',
  children,
}: ListDetailSidebarProps) {
  return (
    <aside className={cn('flex h-full shrink-0 flex-col border-r bg-sidebar', width)}>
      <div className="space-y-3 border-b p-3">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2 font-semibold">
            <Icon className="h-4 w-4 shrink-0" />
            <span className="truncate">{title}</span>
            {count !== undefined ? <Badge variant="secondary">{count}</Badge> : null}
          </div>
          {action}
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={searchPlaceholder}
            className="pl-8"
          />
        </div>
      </div>

      <ScrollArea className="min-h-0 flex-1">
        {isLoading ? (
          <div className="flex h-32 items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : error ? (
          <p className="p-4 text-sm text-destructive">{getErrorMessage(error)}</p>
        ) : isEmpty ? (
          <p className="p-4 text-sm text-muted-foreground">{emptyMessage}</p>
        ) : (
          <nav className="p-2">{children}</nav>
        )}
      </ScrollArea>
    </aside>
  )
}

interface ListDetailSidebarItemProps {
  to: NonNullable<LinkProps['to']>
  params?: LinkProps['params']
  /** Leading slot: avatar or icon. */
  leading?: ReactNode
  primary: ReactNode
  secondary?: ReactNode
  /** Trailing slot: badge, switch, etc. */
  trailing?: ReactNode
}

function ListDetailSidebarItem({
  to,
  params,
  leading,
  primary,
  secondary,
  trailing,
}: ListDetailSidebarItemProps) {
  return (
    <Link
      to={to}
      {...(params !== undefined && { params })}
      activeProps={{ 'data-active': 'true' } as const}
      className="flex items-center gap-3 rounded-md px-2 py-2 text-sm hover:bg-sidebar-accent data-[active=true]:bg-sidebar-accent data-[active=true]:font-medium"
    >
      {leading}
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate font-medium">{primary}</span>
        </span>
        {secondary !== undefined ? (
          <span className="block truncate text-xs text-muted-foreground">{secondary}</span>
        ) : null}
      </span>
      {trailing}
    </Link>
  )
}

export const ListDetailSidebar = Object.assign(ListDetailSidebarRoot, {
  Item: ListDetailSidebarItem,
})
