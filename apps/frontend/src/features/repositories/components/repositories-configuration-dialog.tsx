import { FolderGit2, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { SidebarDialog } from '@/lib/components/ui/sidebar-dialog'
import { useRepositoriesList } from '../hooks/use-repositories'
import { RepositoryConfigCreateSection } from './sections/repository-config-create-section'
import { RepositoryConfigEditSection } from './sections/repository-config-edit-section'

interface RepositoriesConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialRepositoryId?: string
  defaultToCreate?: boolean
}

export function RepositoriesConfigurationDialog({
  open,
  onOpenChange,
  initialRepositoryId,
  defaultToCreate = false,
}: RepositoriesConfigurationDialogProps) {
  const { data: repositories = [], isLoading, refetch } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')
  const [activeSection, setActiveSection] = useState(
    initialRepositoryId || (defaultToCreate ? 'new' : ''),
  )
  const [showNewRepository, setShowNewRepository] = useState(defaultToCreate)

  // Set initial active section when repositories load
  useEffect(() => {
    if (!activeSection && repositories.length > 0 && !initialRepositoryId && !showNewRepository) {
      setActiveSection(repositories[0].id)
    }
  }, [repositories, activeSection, initialRepositoryId, showNewRepository])

  // Create stable sections metadata - only recreate when actual section data changes
  const stableSections = useMemo(() => {
    const filtered = searchQuery
      ? repositories.filter(
          (repo) =>
            repo.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
            repo.path.toLowerCase().includes(searchQuery.toLowerCase()),
        )
      : repositories

    const repoSections = filtered.map((repo) => ({
      id: repo.id,
      label: repo.name,
      icon: <FolderGit2 className="h-4 w-4" />,
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

  return (
    <SidebarDialog open={open} onOpenChange={onOpenChange} className="sm:max-w-[1400px]">
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

      <SidebarDialog.Content title={currentSection?.label}>
        {currentSection?.componentType === 'new' && (
          <RepositoryConfigCreateSection
            key="new-repository"
            onCancel={() => {
              setShowNewRepository(false)
              if (repositories.length > 0) {
                setActiveSection(repositories[0].id)
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
