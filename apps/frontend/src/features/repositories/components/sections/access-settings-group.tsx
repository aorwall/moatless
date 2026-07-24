import type { Scope } from '@moatless/api-client'
import { CheckCircle2, CircleHelp, Loader2, ShieldCheck, TriangleAlert } from 'lucide-react'
import { accessStatusLabel, isAccessFailure } from '@/features/repositories/utils/access-status'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
} from '@/lib/components/configuration'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface AccessTeam {
  id: string
  name: string
  slug: string
}

interface AccessUser {
  id: string
  login: string
  name?: string | null
}

interface AccessSettingsGroupProps {
  scope: Scope
  teamId: string | null | undefined
  ownerUserId: string | null | undefined
  teams: AccessTeam[]
  users: AccessUser[]
  teamsLoading: boolean
  accessStatus: string | undefined
  accessCheckedAt: string | null | undefined
  accessError: string | null | undefined
  isDirty: boolean
  isVerifying: boolean
  onScopeChange: (scope: Scope) => void
  onTeamChange: (teamId: string) => void
  onVerify: () => void
}

function ownerDisplayName(ownerUserId: string | null | undefined, users: AccessUser[]): string {
  if (!ownerUserId) {
    return 'No owner'
  }
  const owner = users.find((user) => user.id === ownerUserId)
  if (!owner) {
    return ownerUserId
  }
  return owner.name ? `${owner.name} (${owner.login})` : owner.login || ownerUserId
}

function formatCheckedAt(value: string | null | undefined): string {
  if (!value) {
    return 'Never checked'
  }
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function credentialSourceLabel(
  scope: Scope,
  ownerUserId: string | null | undefined,
  users: AccessUser[],
  selectedTeam: AccessTeam | undefined,
  teamId: string | null | undefined,
): string {
  if (scope === 'user') {
    return `${ownerDisplayName(ownerUserId, users)}'s Personal Access Token, or their OAuth token when no PAT is set`
  }
  if (scope === 'team') {
    return `${selectedTeam?.name ?? teamId ?? 'Selected team'}'s bound GitHub App installation`
  }
  return 'The globally bound GitHub App installation'
}

export function AccessSettingsGroup({
  scope,
  teamId,
  ownerUserId,
  teams,
  users,
  teamsLoading,
  accessStatus,
  accessCheckedAt,
  accessError,
  isDirty,
  isVerifying,
  onScopeChange,
  onTeamChange,
  onVerify,
}: AccessSettingsGroupProps) {
  const selectedTeam = teams.find((team) => team.id === teamId)
  const statusIsFailure = isAccessFailure(accessStatus)
  const credentialSource = credentialSourceLabel(scope, ownerUserId, users, selectedTeam, teamId)

  return (
    <ConfigurationGroup title="Access">
      <ConfigurationItem
        label="Scope"
        description="Choose who can discover and use this repository"
      >
        <Select value={scope} onValueChange={(value) => onScopeChange(value as Scope)}>
          <SelectTrigger aria-label="Repository access scope" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="global">Global</SelectItem>
            <SelectItem value="team">Team</SelectItem>
            <SelectItem value="user" disabled={!ownerUserId}>
              User
            </SelectItem>
          </SelectContent>
        </Select>
        {!ownerUserId && scope !== 'user' && (
          <p className="mt-2 text-sm text-muted-foreground">
            User scope is unavailable because this repository has no owner.
          </p>
        )}
      </ConfigurationItem>

      {scope === 'team' && (
        <ConfigurationItem
          label="Team"
          description="The team whose members can access this repository"
        >
          <Select value={teamId ?? ''} onValueChange={onTeamChange} disabled={teamsLoading}>
            <SelectTrigger
              aria-label="Repository access team"
              aria-invalid={!teamId}
              className="w-full"
            >
              <SelectValue placeholder={teamsLoading ? 'Loading teams...' : 'Select a team'} />
            </SelectTrigger>
            <SelectContent>
              {teams.map((team) => (
                <SelectItem key={team.id} value={team.id}>
                  {team.name} ({team.slug})
                </SelectItem>
              ))}
              {teamId && !selectedTeam && (
                <SelectItem value={teamId} disabled>
                  Selected team ({teamId})
                </SelectItem>
              )}
            </SelectContent>
          </Select>
          {!teamId && <p className="mt-2 text-sm text-destructive">Select a team to save.</p>}
        </ConfigurationItem>
      )}

      {scope === 'user' && (
        <ConfigurationItem
          label="Owner"
          description="Repository ownership is read-only in this version"
        >
          <ConfigurationInput
            aria-label="Repository owner"
            value={ownerDisplayName(ownerUserId, users)}
            disabled
            className="bg-muted"
          />
        </ConfigurationItem>
      )}

      <ConfigurationItem
        label="GitHub credential"
        description="The persisted repository scope is the only credential source; other scopes are never used as fallbacks"
      >
        <p className="text-sm">{credentialSource}</p>
      </ConfigurationItem>

      <ConfigurationItem
        label="Access status"
        description={`Last checked: ${formatCheckedAt(accessCheckedAt)}`}
      >
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={statusIsFailure ? 'destructive' : 'secondary'}>
              {accessStatus === 'ok' ? (
                <CheckCircle2 />
              ) : statusIsFailure ? (
                <TriangleAlert />
              ) : (
                <CircleHelp />
              )}
              {accessStatusLabel(accessStatus)}
            </Badge>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onVerify}
              disabled={isDirty || isVerifying}
              title={
                isDirty ? 'Save or discard changes before verifying persisted access' : undefined
              }
            >
              {isVerifying ? <Loader2 className="animate-spin" /> : <ShieldCheck />}
              {isVerifying ? 'Verifying...' : 'Verify access'}
            </Button>
          </div>
          {accessError && <p className="text-sm text-destructive">{accessError}</p>}
          {isDirty && (
            <p className="text-sm text-muted-foreground">
              Save or discard changes before verifying access.
            </p>
          )}
        </div>
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
