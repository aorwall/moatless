import { useState } from 'react'

import { useAdminTeamsList } from '@/features/admin/hooks/use-admin-users-teams'
import { GitHubInstallationCard } from '@/features/settings/components/github-installation-card'
import { ConfigurationSectionWrapper } from '@/lib/components/configuration'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

const GLOBAL_SCOPE = 'global'

export function AdminGitHubIntegrationSection() {
  const teamsQuery = useAdminTeamsList()
  const [scope, setScope] = useState(GLOBAL_SCOPE)
  const teamId = scope === GLOBAL_SCOPE ? undefined : scope

  return (
    <ConfigurationSectionWrapper className="p-6">
      <div className="space-y-4">
        <div className="max-w-sm space-y-2">
          <label className="text-sm font-medium" htmlFor="github-installation-scope">
            Scope
          </label>
          <Select value={scope} onValueChange={setScope}>
            <SelectTrigger id="github-installation-scope">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={GLOBAL_SCOPE}>Global</SelectItem>
              {teamsQuery.data?.map((team) => (
                <SelectItem key={team.id} value={team.id}>
                  {team.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Global installations apply to every team; a team-scoped installation only authenticates
            repositories for that team.
          </p>
        </div>
        <GitHubInstallationCard key={scope} {...(teamId !== undefined && { teamId })} />
      </div>
    </ConfigurationSectionWrapper>
  )
}
