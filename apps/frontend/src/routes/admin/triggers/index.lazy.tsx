import { createLazyFileRoute } from '@tanstack/react-router'
import { Bell } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/triggers/')({
  component: TriggersIndexPage,
})

function TriggersIndexPage() {
  return (
    <DetailEmptyState
      icon={Bell}
      title="Select a trigger"
      description="Choose a trigger from the list to view its configuration, or add a new one."
    />
  )
}
