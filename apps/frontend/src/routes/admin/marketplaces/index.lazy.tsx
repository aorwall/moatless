import { createLazyFileRoute } from '@tanstack/react-router'
import { MarketplacesPage } from '@/features/plugins'

export const Route = createLazyFileRoute('/admin/marketplaces/')({
  component: MarketplacesPage,
})
