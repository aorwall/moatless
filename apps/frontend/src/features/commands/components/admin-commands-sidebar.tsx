import type { CommandResponse } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Plus, Terminal } from 'lucide-react'
import { useState } from 'react'
import { useCommandsList } from '@/features/commands/hooks/use-commands'
import { getLucideIcon } from '@/features/commands/utils/lucide-icon'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (command: CommandResponse) => [command.name, command.description]
const sortByName = (a: CommandResponse, b: CommandResponse) => a.name.localeCompare(b.name)

function CommandIcon({ icon }: { icon: string | null | undefined }) {
  const IconComponent = icon ? getLucideIcon(icon) : null
  return IconComponent ? (
    <IconComponent className="h-4 w-4 shrink-0 text-muted-foreground" />
  ) : (
    <Terminal className="h-4 w-4 shrink-0 text-muted-foreground" />
  )
}

export function AdminCommandsSidebar() {
  const { data: commands, isLoading, error } = useCommandsList()
  const [search, setSearch] = useState('')

  const items = useListFilter(commands, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={Terminal}
      title="Commands"
      count={commands?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/commands/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add command</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search commands..."
      isLoading={isLoading}
      error={error}
      isEmpty={items.length === 0}
      emptyMessage="No commands yet"
    >
      {items.map((command) => (
        <ListDetailSidebar.Item
          key={command.name}
          to="/admin/commands/$name"
          params={{ name: command.name }}
          leading={<CommandIcon icon={command.icon} />}
          primary={command.name}
          secondary={command.description}
          trailing={
            command.active === false ? (
              <Badge variant="outline" className="h-4 px-1 text-[10px]">
                off
              </Badge>
            ) : null
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
