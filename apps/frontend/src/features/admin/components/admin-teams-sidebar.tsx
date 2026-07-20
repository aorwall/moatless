import { Link } from '@tanstack/react-router'
import { Building2, Plus } from 'lucide-react'
import { useState } from 'react'
import type { AdminTeam } from '@/features/admin/hooks/use-admin-users-teams'
import { useAdminTeamsList } from '@/features/admin/hooks/use-admin-users-teams'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (team: AdminTeam) => [team.name, team.slug]
const sortByName = (a: AdminTeam, b: AdminTeam) => a.name.localeCompare(b.name)

export function AdminTeamsSidebar() {
  const { data, isLoading, error } = useAdminTeamsList()
  const [search, setSearch] = useState('')
  const teams = useListFilter(data, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={Building2}
      title="Teams"
      count={data?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/teams/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add team</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search teams..."
      isLoading={isLoading}
      error={error}
      isEmpty={teams.length === 0}
      emptyMessage="No teams found"
    >
      {teams.map((team) => (
        <ListDetailSidebar.Item
          key={team.id}
          to="/admin/teams/$teamId"
          params={{ teamId: team.id }}
          primary={team.name}
          secondary={team.slug}
          trailing={
            team.isDefault ? (
              <Badge variant="outline" className="h-4 px-1 text-[10px]">
                default
              </Badge>
            ) : null
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
