import { createLazyFileRoute } from '@tanstack/react-router'
import { Boxes } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/workspaces/')({
  component: WorkspacesIndexPage,
})

function WorkspacesIndexPage() {
  return (
    <DetailEmptyState
      icon={Boxes}
      title="Select a workspace"
      description="Choose a workspace from the list to view and edit its configuration and repositories."
    />
  )
}
