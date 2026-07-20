import { createLazyFileRoute } from '@tanstack/react-router'
import { FolderGit2 } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/repositories/')({
  component: RepositoriesIndexPage,
})

function RepositoriesIndexPage() {
  return (
    <DetailEmptyState
      icon={FolderGit2}
      title="Select a repository"
      description="Choose a repository from the list to view and edit its configuration, or add a new one."
    />
  )
}
