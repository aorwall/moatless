import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminGitHubIntegrationSection } from '@/features/settings/components/admin-github-integration-section'

export const Route = createLazyFileRoute('/admin/integrations/github')({
  component: AdminGitHubIntegrationPage,
})

function AdminGitHubIntegrationPage() {
  return <AdminGitHubIntegrationSection />
}
