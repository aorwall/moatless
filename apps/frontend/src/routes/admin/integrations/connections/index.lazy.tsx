import { createLazyFileRoute } from '@tanstack/react-router'
import { Plug } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/integrations/connections/')({
  component: ConnectionsIndexPage,
})

function ConnectionsIndexPage() {
  return (
    <DetailEmptyState
      icon={Plug}
      title="Select a connection"
      description="Choose a connection from the list to view its details."
    />
  )
}
