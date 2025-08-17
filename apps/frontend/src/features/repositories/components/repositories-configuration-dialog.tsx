import { FolderGit2, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { useRepositoriesList } from '../hooks/use-repositories'
import { RepositoryConfigCreateSection } from './sections/repository-config-create-section'
import { RepositoryConfigEditSection } from './sections/repository-config-edit-section'

interface RepositoriesConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialRepositoryId?: string
}

export function RepositoriesConfigurationDialog({
  open,
  onOpenChange,
  initialRepositoryId,
}: RepositoriesConfigurationDialogProps) {
  const { data: repositories = [], isLoading, refetch } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')
  const [activeSection, setActiveSection] = useState(initialRepositoryId || '')
  const [showNewRepository, setShowNewRepository] = useState(false)

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

  // Create stable component rendering with proper memoization
  const activeComponent = useMemo(() => {
    const currentSection = stableSections.find((s) => s.id === activeSection)
    if (!currentSection) return null

    if (currentSection.componentType === 'new') {
      return (
        <RepositoryConfigCreateSection
          key="new-repository" // Stable key
          onCancel={() => {
            setShowNewRepository(false)
            // Switch to first repository or empty state
            if (repositories.length > 0) {
              setActiveSection(repositories[0].id)
            }
          }}
          onSuccess={async (repositoryId) => {
            setShowNewRepository(false)
            // Refresh repositories list
            await refetch()
            // Switch to the newly created repository
            setActiveSection(repositoryId)
          }}
        />
      )
    }

    if (currentSection.componentType === 'repository' && currentSection.repositoryId) {
      return (
        <RepositoryConfigEditSection
          key={currentSection.repositoryId} // Stable key based on repository ID
          repositoryId={currentSection.repositoryId}
          onDeleted={async () => {
            // Refresh repositories list
            await refetch()
            // Switch to first repository or close dialog if none left
            const remainingRepos = repositories.filter((r) => r.id !== currentSection.repositoryId)
            if (remainingRepos.length > 0) {
              setActiveSection(remainingRepos[0].id)
            } else {
              setShowNewRepository(false)
              setActiveSection('')
              // Close dialog if no repositories left
              if (remainingRepos.length === 0) {
                onOpenChange(false)
              }
            }
          }}
        />
      )
    }

    return null
  }, [stableSections, activeSection, repositories, refetch, onOpenChange])

  const handleAddRepository = () => {
    setShowNewRepository(true)
    setActiveSection('new')
    setSearchQuery('') // Clear search when adding new
  }

  // Create sidebar footer with Add Repository button
  const sidebarFooter = (
    <Button className="w-full" size="sm" onClick={handleAddRepository}>
      <Plus className="h-4 w-4 mr-2" />
      Add Repository
    </Button>
  )

  if (isLoading && !open) {
    return null
  }

  return (
    <ResponsiveConfigurationDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Repositories"
      description="Configure your repositories"
      sections={stableSections.map((section) => ({
        id: section.id,
        label: section.label,
        icon: section.icon,
        component: section.id === activeSection ? activeComponent : null,
      }))}
      activeSection={activeSection}
      onSectionChange={setActiveSection}
      searchable
      searchPlaceholder="Search repositories..."
      searchValue={searchQuery}
      onSearchChange={setSearchQuery}
      sidebarFooter={sidebarFooter}
      className="sm:max-w-[1400px]" // Wider for repository configuration
    />
  )
}
