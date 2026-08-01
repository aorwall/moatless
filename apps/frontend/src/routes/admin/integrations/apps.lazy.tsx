import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminAdapterAppsPage } from '@/features/integrations/components/admin-adapter-apps-page'

export const Route = createLazyFileRoute('/admin/integrations/apps')({
  component: AdminAdapterAppsPage,
})
