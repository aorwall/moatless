import { createLazyFileRoute } from '@tanstack/react-router'
import { Terminal } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/commands/')({
  component: CommandsIndexPage,
})

function CommandsIndexPage() {
  return (
    <DetailEmptyState
      icon={Terminal}
      title="Select a command"
      description="Choose a command from the list to view and edit it."
    />
  )
}
