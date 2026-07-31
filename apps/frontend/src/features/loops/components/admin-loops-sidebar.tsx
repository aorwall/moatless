import type { Loop } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Plus, Repeat } from 'lucide-react'
import { useState } from 'react'
import { useLoops } from '@/features/loops/hooks/use-loops'
import { getSourceLabel } from '@/features/loops/utils/loop-display'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

// Searchable by what the Loop is called and by where it listens, so typing
// "slack" or a channel id finds it without knowing its name.
const getSearchText = (loop: Loop) => [loop.name, getSourceLabel(loop)]
const sortByLabel = (a: Loop, b: Loop) => a.name.localeCompare(b.name)

export function AdminLoopsSidebar() {
  const { data: loops, isLoading, error } = useLoops()
  const [search, setSearch] = useState('')

  const items = useListFilter(loops, search, getSearchText, sortByLabel)

  return (
    <ListDetailSidebar
      icon={Repeat}
      title="Loops"
      count={loops?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/loops/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add loop</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search loops..."
      isLoading={isLoading}
      error={error}
      isEmpty={items.length === 0}
      emptyMessage="No loops yet"
    >
      {items.map((loop) => (
        <ListDetailSidebar.Item
          key={loop.id}
          to="/admin/loops/$loop-id"
          params={{ 'loop-id': loop.id }}
          primary={
            <span className={loop.active ? undefined : 'text-muted-foreground'}>{loop.name}</span>
          }
          secondary={getSourceLabel(loop)}
          trailing={
            <span className="flex items-center gap-1">
              <Badge variant="outline" className="h-4 px-1 text-[10px]">
                {loop.deleted
                  ? 'deleted'
                  : loop.executionState === 'awaiting_approval'
                    ? 'approval'
                    : loop.executionState}
              </Badge>
              <Badge variant="outline" className="h-4 px-1 text-[10px]">
                {loop.syncedFromGit ? 'git' : 'manual'}
              </Badge>
            </span>
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
