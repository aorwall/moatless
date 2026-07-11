import { LayoutGrid, Table2, X } from 'lucide-react'
import { useUsersList } from '@/features/auth/hooks/use-users'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTagsList } from '@/features/tags/hooks/use-tags'
import { Button } from '@/lib/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'

const STATUS_OPTIONS = ['open', 'error', 'closed'] as const

export interface DashboardSearchParams {
  view?: 'cards' | 'table' | undefined
  tag?: string | undefined
  createdBy?: string | undefined
  repositoryId?: string | undefined
  status?: 'all' | 'open' | 'error' | 'closed' | undefined
  page?: number | undefined
}

interface TaskFiltersProps {
  view: 'cards' | 'table'
  tag?: string | undefined
  createdBy?: string | undefined
  repositoryId?: string | undefined
  status?: DashboardSearchParams['status']
  onSearchChange: (params: DashboardSearchParams) => void
}

export function TaskFilters({
  view,
  tag,
  createdBy,
  repositoryId,
  status,
  onSearchChange,
}: TaskFiltersProps) {
  const { data: tagsData } = useTagsList()
  const { data: usersData } = useUsersList()
  const { data: repositories } = useRepositoriesList()

  const hasActiveFilters =
    tag || (createdBy && createdBy !== 'all') || repositoryId || (status && status !== 'all')

  function updateFilter(key: keyof DashboardSearchParams, value: string | undefined) {
    const current: DashboardSearchParams = {
      view: view !== 'cards' ? view : undefined,
      tag,
      createdBy,
      repositoryId,
      status: status as DashboardSearchParams['status'],
    }
    const updated: DashboardSearchParams = { ...current, [key]: value }
    // Reset page when filters change (not view)
    if (key !== 'view') {
      updated.page = undefined
    }
    onSearchChange(updated)
  }

  function clearFilters() {
    onSearchChange({ view: view !== 'cards' ? view : undefined, status: 'all', createdBy: 'all' })
  }

  return (
    <div className="flex items-center gap-3 flex-wrap">
      {/* View toggle */}
      <ToggleGroup
        type="single"
        value={view}
        onValueChange={(value) => {
          if (value) {
            updateFilter('view', value === 'cards' ? undefined : value)
          }
        }}
        variant="outline"
        size="sm"
      >
        <ToggleGroupItem value="cards" aria-label="Card view">
          <LayoutGrid className="h-4 w-4" />
        </ToggleGroupItem>
        <ToggleGroupItem value="table" aria-label="Table view">
          <Table2 className="h-4 w-4" />
        </ToggleGroupItem>
      </ToggleGroup>

      {/* Tag filter */}
      <Select
        value={tag ?? ''}
        onValueChange={(value) => updateFilter('tag', value === '__all__' ? undefined : value)}
      >
        <SelectTrigger size="sm" className="min-w-[120px]">
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

      {/* User filter */}
      <Select
        value={createdBy === 'all' ? '__all__' : (createdBy ?? '')}
        onValueChange={(value) => updateFilter('createdBy', value === '__all__' ? 'all' : value)}
      >
        <SelectTrigger size="sm" className="min-w-[120px]">
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

      {/* Repository filter */}
      <Select
        value={repositoryId ?? ''}
        onValueChange={(value) =>
          updateFilter('repositoryId', value === '__all__' ? undefined : value)
        }
      >
        <SelectTrigger size="sm" className="min-w-[120px]">
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

      {/* Status filter */}
      <Select
        value={status === 'all' ? '__all__' : (status ?? '')}
        onValueChange={(value) => updateFilter('status', value === '__all__' ? 'all' : value)}
      >
        <SelectTrigger size="sm" className="min-w-[100px]">
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

      {/* Clear filters */}
      {hasActiveFilters && (
        <Button variant="ghost" size="sm" onClick={clearFilters} className="h-8 px-2 text-xs">
          <X className="h-3 w-3 mr-1" />
          Clear
        </Button>
      )}
    </div>
  )
}
