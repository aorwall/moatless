import type { AdapterConnectionResponse, Loop } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Plus, Repeat } from 'lucide-react'
import { useState } from 'react'
import { useIntegrationConnections } from '@/features/integrations/hooks/use-integrations'
import { useLoops } from '@/features/loops/hooks/use-loops'
import {
  getLoopKindLabel,
  getSourceLabel,
  isAdapterEventSource,
  isScheduleSource,
} from '@/features/loops/utils/loop-display'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (loop: Loop) => [getSourceLabel(loop), loop.name, loop.kind]
const sortByLabel = (a: Loop, b: Loop) => getSourceLabel(a).localeCompare(getSourceLabel(b))

function sourceSubtitle(
  loop: Loop,
  connectionsById: Map<string, AdapterConnectionResponse>,
): string {
  const source = loop.source
  if (isAdapterEventSource(source)) {
    return connectionsById.get(source.connectionId)?.externalAccountId ?? 'Missing connection'
  }
  if (isScheduleSource(source)) {
    return source.cronExpression
  }
  return 'Manual'
}

export function AdminLoopsSidebar() {
  const { data: loops, isLoading, error } = useLoops()
  const { data: connections } = useIntegrationConnections()
  const [search, setSearch] = useState('')

  const connectionsById = new Map(
    (connections ?? []).map((connection) => [connection.id, connection]),
  )
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
            <span className={loop.active ? undefined : 'text-muted-foreground'}>
              {getSourceLabel(loop)}
            </span>
          }
          secondary={sourceSubtitle(loop, connectionsById)}
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
              <Badge variant="secondary" className="h-4 px-1 text-[10px]">
                {getLoopKindLabel(loop)}
              </Badge>
            </span>
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
