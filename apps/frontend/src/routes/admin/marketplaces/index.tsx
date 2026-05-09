import { createFileRoute } from '@tanstack/react-router'
import { MarketplacesPage } from '@/features/plugins'

export const Route = createFileRoute('/admin/marketplaces/')({
  component: MarketplacesPage,
})
