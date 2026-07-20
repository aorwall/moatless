import { createLazyFileRoute } from '@tanstack/react-router'
import { Building2 } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/teams/')({
  component: TeamsIndexPage,
})

function TeamsIndexPage() {
  return (
    <DetailEmptyState
      icon={Building2}
      title="Select a team"
      description="Choose a team from the list to manage its details and members, or add a new one."
    />
  )
}
