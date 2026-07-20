import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminMarketplaceDetail } from '@/features/plugins/components/admin-marketplace-detail'

export const Route = createLazyFileRoute('/admin/marketplaces/$marketplace-id/')({
  component: MarketplaceDetailPage,
})

function MarketplaceDetailPage() {
  const params = Route.useParams()
  return <AdminMarketplaceDetail marketplaceId={params['marketplace-id']} />
}
