import { createLazyFileRoute } from '@tanstack/react-router'
import { UserRound } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/users/')({
  component: AdminUsersIndexPage,
})

function AdminUsersIndexPage() {
  return (
    <DetailEmptyState
      icon={UserRound}
      title="Select a user"
      description="Choose a user from the list to view and edit their profile."
    />
  )
}
