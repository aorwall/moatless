import type { WorkspaceResponse } from '@moatless/api-client'
import { Boxes, FileStack, FolderGit2, GitBranch, Layers, PencilLine, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useWorkspacesList } from '@/features/workspaces/hooks/use-workspaces'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (ws: WorkspaceResponse) => [ws.name]
const sortByName = (a: WorkspaceResponse, b: WorkspaceResponse) =>
  a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })

// Leading icon reflects the workspace's derived kind.
function getWorkspaceIcon(ws: WorkspaceResponse) {
  if (ws.deleted) {
    return <Trash2 className="h-4 w-4 shrink-0 text-muted-foreground" />
  }
  if (ws.kind === 'multi') {
    return <Layers className="h-4 w-4 shrink-0 text-muted-foreground" />
  }
  if (ws.kind === 'blank') {
    return <FileStack className="h-4 w-4 shrink-0 text-muted-foreground" />
  }
  return <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
}

function getSecondary(ws: WorkspaceResponse): string {
  if (ws.deleted) {
    return 'Deleted Git override'
  }
  const count = ws.repos.length
  return count === 1 ? '1 repository' : `${count} repositories`
}

// Git provenance gets a trailing icon: a branch while the workspace is synced
// (read-only), a pencil once it has been overridden and is edited here instead.
function getWorkspaceTrailing(ws: WorkspaceResponse) {
  if (ws.source === 'git' || ws.syncedFromGit) {
    return <GitBranch className="h-3.5 w-3.5 text-muted-foreground" />
  }
  if (ws.sourceRepositoryId) {
    return <PencilLine className="h-3.5 w-3.5 text-muted-foreground" />
  }
  return
}

export function AdminWorkspacesSidebar() {
  const { data, isLoading, error } = useWorkspacesList()
  const [search, setSearch] = useState('')
  const workspaces = useListFilter(data, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={Boxes}
      title="Workspaces"
      count={data?.length}
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search workspaces..."
      isLoading={isLoading}
      error={error}
      isEmpty={workspaces.length === 0}
      emptyMessage="No workspaces found"
    >
      {workspaces.map((ws) => (
        <ListDetailSidebar.Item
          key={ws.id}
          to="/admin/workspaces/$workspaceId"
          params={{ workspaceId: ws.id }}
          leading={getWorkspaceIcon(ws)}
          primary={ws.name}
          secondary={getSecondary(ws)}
          trailing={getWorkspaceTrailing(ws)}
        />
      ))}
    </ListDetailSidebar>
  )
}
