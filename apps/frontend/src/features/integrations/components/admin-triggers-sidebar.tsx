import type { AdapterConnectionResponse, Loop } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Bell, Plus } from 'lucide-react'
import { useState } from 'react'
import {
  useIntegrationConnections,
  useTriggers,
} from '@/features/integrations/hooks/use-integrations'
import {
  getSourceLabel,
  getTriggerKindLabel,
  isAdapterEventSource,
  isScheduleSource,
} from '@/features/integrations/utils/trigger-display'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (trigger: Loop) => [getSourceLabel(trigger), trigger.name, trigger.kind]
const sortByLabel = (a: Loop, b: Loop) => getSourceLabel(a).localeCompare(getSourceLabel(b))

function sourceSubtitle(
  trigger: Loop,
  connectionsById: Map<string, AdapterConnectionResponse>,
): string {
  const source = trigger.source
  if (isAdapterEventSource(source)) {
    return connectionsById.get(source.connectionId)?.externalAccountId ?? 'Missing connection'
  }
  if (isScheduleSource(source)) {
    return source.cronExpression
  }
  return 'Manual'
}

export function AdminTriggersSidebar() {
  const { data: triggers, isLoading, error } = useTriggers()
  const { data: connections } = useIntegrationConnections()
  const [search, setSearch] = useState('')

  const connectionsById = new Map(
    (connections ?? []).map((connection) => [connection.id, connection]),
  )
  const items = useListFilter(triggers, search, getSearchText, sortByLabel)

  return (
    <ListDetailSidebar
      icon={Bell}
      title="Triggers"
      count={triggers?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/triggers/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add trigger</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search triggers..."
      isLoading={isLoading}
      error={error}
      isEmpty={items.length === 0}
      emptyMessage="No triggers yet"
    >
      {items.map((trigger) => (
        <ListDetailSidebar.Item
          key={trigger.id}
          to="/admin/triggers/$trigger-id"
          params={{ 'trigger-id': trigger.id }}
          primary={
            <span className={trigger.active ? undefined : 'text-muted-foreground'}>
              {getSourceLabel(trigger)}
            </span>
          }
          secondary={sourceSubtitle(trigger, connectionsById)}
          trailing={
            <span className="flex items-center gap-1">
              {trigger.active ? null : (
                <Badge variant="outline" className="h-4 px-1 text-[10px]">
                  off
                </Badge>
              )}
              <Badge variant="secondary" className="h-4 px-1 text-[10px]">
                {getTriggerKindLabel(trigger)}
              </Badge>
            </span>
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
