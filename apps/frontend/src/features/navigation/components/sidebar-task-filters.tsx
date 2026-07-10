import { ListFilter, X } from 'lucide-react'
import { useUsersList } from '@/features/auth/hooks/use-users'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTagsList } from '@/features/tags/hooks/use-tags'
import { Button } from '@/lib/components/ui/button'
import { Label } from '@/lib/components/ui/label'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { SidebarGroupAction } from '@/lib/components/ui/sidebar'
import { Switch } from '@/lib/components/ui/switch'

const STATUS_OPTIONS = ['open', 'error', 'closed'] as const

/**
 * Filters applied to the sidebar task list. Mirrors the task dashboard filters
 * so the sidebar can surface non-followed / non-open ("not running") tasks too.
 */
export interface SidebarTaskFilterValues {
  /** `'all'` clears the status filter (open + error + closed). */
  status: 'all' | 'open' | 'error' | 'closed'
  /** Tag name, or `undefined` for all tags. */
  tag?: string
  /** Creator user id, or `'all'` for all users. */
  createdBy?: string
  /** Repository id, or `undefined` for all repositories. */
  repositoryId?: string
  /** When true, only show tasks the current user follows. */
  onlyFollowed: boolean
}

/** Default sidebar view: open tasks the user follows (the personal task list). */
export const DEFAULT_SIDEBAR_FILTERS: SidebarTaskFilterValues = {
  status: 'open',
  createdBy: 'all',
  onlyFollowed: true,
}

/** Whether the given filters differ from the defaults (drives the active dot). */
export function hasActiveSidebarFilters(filters: SidebarTaskFilterValues): boolean {
  return (
    filters.status !== DEFAULT_SIDEBAR_FILTERS.status ||
    !!filters.tag ||
    (!!filters.createdBy && filters.createdBy !== 'all') ||
    !!filters.repositoryId ||
    filters.onlyFollowed !== DEFAULT_SIDEBAR_FILTERS.onlyFollowed
  )
}

interface SidebarTaskFiltersProps {
  filters: SidebarTaskFilterValues
  onChange: (filters: SidebarTaskFilterValues) => void
}

export function SidebarTaskFilters({ filters, onChange }: SidebarTaskFiltersProps) {
  const { data: tagsData } = useTagsList()
  const { data: usersData } = useUsersList()
  const { data: repositories } = useRepositoriesList()

  const active = hasActiveSidebarFilters(filters)

  function update<K extends keyof SidebarTaskFilterValues>(
    key: K,
    value: SidebarTaskFilterValues[K],
  ) {
    onChange({ ...filters, [key]: value })
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <SidebarGroupAction aria-label="Filter tasks" title="Filter tasks">
          <ListFilter />
          {active && (
            <span className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-sky-500" />
          )}
        </SidebarGroupAction>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-64 space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium">Filter tasks</span>
          {active && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
              onClick={() => onChange(DEFAULT_SIDEBAR_FILTERS)}
            >
              <X className="h-3 w-3 mr-1" />
              Reset
            </Button>
          )}
        </div>

        {/* Status */}
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Status</Label>
          <Select
            value={filters.status === 'all' ? '__all__' : filters.status}
            onValueChange={(value) =>
              update('status', value === '__all__' ? 'all' : (value as 'open' | 'error' | 'closed'))
            }
          >
            <SelectTrigger size="sm" className="w-full">
              <SelectValue placeholder="Status" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All statuses</SelectItem>
              {STATUS_OPTIONS.map((s) => (
                <SelectItem key={s} value={s}>
                  {s.charAt(0).toUpperCase() + s.slice(1)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Tag */}
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Tag</Label>
          <Select
            value={filters.tag ?? '__all__'}
            onValueChange={(value) => update('tag', value === '__all__' ? undefined : value)}
          >
            <SelectTrigger size="sm" className="w-full">
              <SelectValue placeholder="Tag" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All tags</SelectItem>
              {tagsData?.map((t) => (
                <SelectItem key={t.tagId} value={t.name}>
                  {t.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Created by */}
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Created by</Label>
          <Select
            value={filters.createdBy === 'all' ? '__all__' : (filters.createdBy ?? '__all__')}
            onValueChange={(value) => update('createdBy', value === '__all__' ? 'all' : value)}
          >
            <SelectTrigger size="sm" className="w-full">
              <SelectValue placeholder="User" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All users</SelectItem>
              {usersData?.users.map((u) => (
                <SelectItem key={u.id} value={u.id}>
                  {u.login || u.name || u.id}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Repository */}
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Repository</Label>
          <Select
            value={filters.repositoryId ?? '__all__'}
            onValueChange={(value) =>
              update('repositoryId', value === '__all__' ? undefined : value)
            }
          >
            <SelectTrigger size="sm" className="w-full">
              <SelectValue placeholder="Repository" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">All repos</SelectItem>
              {repositories?.map((r) => (
                <SelectItem key={r.id} value={r.id}>
                  {r.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Only followed */}
        <div className="flex items-center justify-between pt-1">
          <Label htmlFor="sidebar-only-followed" className="text-xs text-muted-foreground">
            Only tasks I follow
          </Label>
          <Switch
            id="sidebar-only-followed"
            checked={filters.onlyFollowed}
            onCheckedChange={(checked) => update('onlyFollowed', checked)}
          />
        </div>
      </PopoverContent>
    </Popover>
  )
}
