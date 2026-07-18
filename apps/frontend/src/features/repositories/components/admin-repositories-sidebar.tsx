import type { RepositoryResponse } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { FileCode, FolderGit2, Plus } from 'lucide-react'
import { useState } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { getRepositoryLocation } from '@/features/repositories/utils/repository.utils'
import { Github } from '@/lib/components/ui/brand-icons'
import { Button } from '@/lib/components/ui/button'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (repo: RepositoryResponse) => [repo.name, getRepositoryLocation(repo)]
const sortByName = (a: RepositoryResponse, b: RepositoryResponse) =>
  a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })

// Leading icon for a repository row, based on its kind.
function getRepositoryIcon(repo: RepositoryResponse) {
  if (repo.kind === 'template') {
    return <FileCode className="h-4 w-4 shrink-0 text-muted-foreground" />
  }
  return <Github className="h-4 w-4 shrink-0 text-muted-foreground" />
}

export function AdminRepositoriesSidebar() {
  const { data, isLoading, error } = useRepositoriesList()
  const [search, setSearch] = useState('')
  const repositories = useListFilter(data, search, getSearchText, sortByName)

  return (
    <ListDetailSidebar
      icon={FolderGit2}
      title="Repositories"
      count={data?.length}
      action={
        <Button size="sm" variant="ghost" asChild>
          <Link to="/admin/repositories/new">
            <Plus className="h-4 w-4" />
            <span className="sr-only">Add repository</span>
          </Link>
        </Button>
      }
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search repositories..."
      isLoading={isLoading}
      error={error}
      isEmpty={repositories.length === 0}
      emptyMessage="No repositories found"
    >
      {repositories.map((repo) => (
        <ListDetailSidebar.Item
          key={repo.id}
          to="/admin/repositories/$repositoryId"
          params={{ repositoryId: repo.id }}
          leading={getRepositoryIcon(repo)}
          primary={repo.name}
          secondary={getRepositoryLocation(repo) || undefined}
        />
      ))}
    </ListDetailSidebar>
  )
}
