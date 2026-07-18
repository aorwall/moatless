import { createLazyFileRoute } from '@tanstack/react-router'
import { Package } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/marketplaces/')({
  component: MarketplacesIndexPage,
})

function MarketplacesIndexPage() {
  return (
    <DetailEmptyState
      icon={Package}
      title="Select a marketplace"
      description="Choose a marketplace from the list to view its details and plugins."
    />
  )
}
