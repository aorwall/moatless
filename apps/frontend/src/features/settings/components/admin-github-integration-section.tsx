import { GitHubAppsCard } from '@/features/settings/components/github-apps-card'
import { GitHubInstallationCard } from '@/features/settings/components/github-installation-card'
import { ConfigurationSectionWrapper } from '@/lib/components/configuration'

export function AdminGitHubIntegrationSection() {
  return (
    <ConfigurationSectionWrapper className="space-y-6 p-6">
      <GitHubAppsCard />
      <GitHubInstallationCard />
    </ConfigurationSectionWrapper>
  )
}
