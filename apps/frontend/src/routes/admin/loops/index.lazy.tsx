import { createLazyFileRoute } from '@tanstack/react-router'
import { Repeat } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/loops/')({
  component: LoopsIndexPage,
})

function LoopsIndexPage() {
  return (
    <DetailEmptyState
      icon={Repeat}
      title="Select a loop"
      description="Choose a loop from the list to view its configuration, or add a new one."
    />
  )
}
