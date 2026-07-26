import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminPluginDetail } from '@/features/plugins/components/admin-plugin-detail'

export const Route = createLazyFileRoute('/admin/plugins/$plugin-id/')({
  component: PluginDetailPage,
})

function PluginDetailPage() {
  const params = Route.useParams()
  return <AdminPluginDetail pluginId={params['plugin-id']} />
}
