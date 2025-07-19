import { useNavigate } from '@tanstack/react-router'
import { FolderGit2, Plus } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { useRepositoriesList } from '../hooks/use-repositories'
import { RepositoryAllConfigSection } from './sections/repository-all-config-section'

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
  const navigate = useNavigate()
  const { data: repositories = [], isLoading } = useRepositoriesList()
  const [searchQuery, setSearchQuery] = useState('')
  const [activeSection, setActiveSection] = useState(initialRepositoryId || '')

  // Set initial active section when repositories load
  useEffect(() => {
    if (!activeSection && repositories.length > 0 && !initialRepositoryId) {
      setActiveSection(repositories[0].id)
    }
  }, [repositories, activeSection, initialRepositoryId])

  // Filter and map repositories to sections
  const sections = useMemo(() => {
    const filtered = searchQuery
      ? repositories.filter(
          (repo) =>
            repo.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
            repo.path.toLowerCase().includes(searchQuery.toLowerCase()),
        )
      : repositories

    return filtered.map((repo) => ({
      id: repo.id,
      label: repo.name,
      icon: <FolderGit2 className="h-4 w-4" />,
      component: <RepositoryAllConfigSection repositoryId={repo.id} />,
    }))
  }, [repositories, searchQuery])

  const handleAddRepository = () => {
    onOpenChange(false)
    navigate({ to: '/repositories/new' })
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
      sections={sections}
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
