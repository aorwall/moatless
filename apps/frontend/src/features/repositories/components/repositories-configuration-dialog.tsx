import type { RepositoryResponse } from '@moatless/api-client'
import { FileCode, Github, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import {
  getRepositoryLocation,
  RepositoryConfigCreateSection,
  RepositoryConfigEditSection,
  useRepositoriesList,
} from '@/features/repositories'
import { Button } from '@/lib/components/ui/button'
import { SidebarDialog } from '@/lib/components/ui/sidebar-dialog'

interface RepositoriesConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialRepositoryId?: string
  defaultToCreate?: boolean
}

// Helper function to get repository icon based on kind
function getRepositoryIcon(repo: RepositoryResponse) {
  if (repo.kind === 'remote') {
    return <Github className="h-4 w-4" />
  }
  if (repo.kind === 'template') {
    return <FileCode className="h-4 w-4" />
  }
  return <Github className="h-4 w-4" />
}

export function RepositoriesConfigurationDialog({
  open,
  onOpenChange,
  initialRepositoryId,
  defaultToCreate = false,
}: RepositoriesConfigurationDialogProps) {
  const { data: repositories = [], isLoading, refetch } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')

  // Auto-show create section when no repositories exist
  const shouldShowCreate = repositories.length === 0 || defaultToCreate
  const [activeSection, setActiveSection] = useState(
    initialRepositoryId || (shouldShowCreate ? 'new' : ''),
  )
  const [showNewRepository, setShowNewRepository] = useState(shouldShowCreate)

  // Set initial active section when repositories load
  useEffect(() => {
    if (repositories.length === 0) {
      // No repos - always show create section
      setShowNewRepository(true)
      setActiveSection('new')
    } else if (!(activeSection || initialRepositoryId || showNewRepository)) {
      // Has repos but no active section - select first repo
      setActiveSection(repositories[0].id)
    }
  }, [repositories, activeSection, initialRepositoryId, showNewRepository])

  // Create stable sections metadata - only recreate when actual section data changes
  const stableSections = useMemo(() => {
    const filtered = searchQuery
      ? repositories.filter((repo) => {
          const location = getRepositoryLocation(repo)
          return (
            repo.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
            location.toLowerCase().includes(searchQuery.toLowerCase())
          )
        })
      : repositories

    // Sort repositories alphabetically by name
    const sortedRepos = [...filtered].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
    )

    const repoSections = sortedRepos.map((repo) => ({
      id: repo.id,
      label: repo.name,
      icon: getRepositoryIcon(repo),
      // Store only metadata, not component instances
      componentType: 'repository' as const,
      repositoryId: repo.id,
    }))

    // Add "new repository" section if active
    if (showNewRepository && !searchQuery) {
      return [
        {
          id: 'new',
          label: 'New Repository',
          icon: <Plus className="h-4 w-4" />,
          componentType: 'new' as const,
        },
        ...repoSections,
      ]
    }

    return repoSections
  }, [repositories, searchQuery, showNewRepository])

  const handleAddRepository = () => {
    setShowNewRepository(true)
    setActiveSection('new')
    setSearchQuery('') // Clear search when adding new
  }

  if (isLoading && !open) {
    return null
  }

  const currentSection = stableSections.find((s) => s.id === activeSection)

  const hasRepositories = repositories.length > 0

  return (
    <SidebarDialog open={open} onOpenChange={onOpenChange} className="sm:max-w-[1400px]">
      {hasRepositories && (
        <SidebarDialog.Sidebar title="Repositories" description="Configure your repositories">
          <SidebarDialog.Search
            value={searchQuery}
            onChange={setSearchQuery}
            placeholder="Search repositories..."
          />

          <SidebarDialog.Nav>
            {stableSections.map(({ id, label, icon }) => (
              <SidebarDialog.NavItem
                key={id}
                active={activeSection === id}
                onClick={() => setActiveSection(id)}
                icon={icon}
              >
                {label}
              </SidebarDialog.NavItem>
            ))}
          </SidebarDialog.Nav>

          <SidebarDialog.Footer>
            <Button className="w-full" size="sm" onClick={handleAddRepository}>
              <Plus className="h-4 w-4 mr-2" />
              Add Repository
            </Button>
          </SidebarDialog.Footer>
        </SidebarDialog.Sidebar>
      )}

      <SidebarDialog.Content title={currentSection?.label || 'New Repository'}>
        {currentSection?.componentType === 'new' && (
          <RepositoryConfigCreateSection
            key="new-repository"
            onCancel={() => {
              setShowNewRepository(false)
              if (repositories.length > 0) {
                setActiveSection(repositories[0].id)
              } else {
                // No repos to show - close the dialog
                onOpenChange(false)
              }
            }}
            onSuccess={async (repositoryId) => {
              setShowNewRepository(false)
              await refetch()
              setActiveSection(repositoryId)
            }}
          />
        )}
        {currentSection?.componentType === 'repository' && currentSection.repositoryId && (
          <RepositoryConfigEditSection
            key={currentSection.repositoryId}
            repositoryId={currentSection.repositoryId}
            onDeleted={async () => {
              await refetch()
              const remainingRepos = repositories.filter(
                (r) => r.id !== currentSection.repositoryId,
              )
              if (remainingRepos.length > 0) {
                setActiveSection(remainingRepos[0].id)
              } else {
                setShowNewRepository(false)
                setActiveSection('')
                onOpenChange(false)
              }
            }}
          />
        )}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
