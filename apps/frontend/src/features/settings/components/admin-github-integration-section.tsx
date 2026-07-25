import { GitHubInstallationCard } from '@/features/settings/components/github-installation-card'
import { ConfigurationSectionWrapper } from '@/lib/components/configuration'

export function AdminGitHubIntegrationSection() {
  return (
    <ConfigurationSectionWrapper className="p-6">
      <GitHubInstallationCard />
    </ConfigurationSectionWrapper>
  )
}
