import type { Scope } from '@moatless/api-client'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
} from '@/lib/components/configuration'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface AccessUser {
  id: string
  login: string
  name?: string | null
}

interface AccessSettingsGroupProps {
  scope: Scope
  ownerUserId: string | null | undefined
  users: AccessUser[]
  onScopeChange: (scope: Scope) => void
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

export function AccessSettingsGroup({
  scope,
  ownerUserId,
  users,
  onScopeChange,
}: AccessSettingsGroupProps) {
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
        description="Whoever opens this repository reaches it with their own connected account"
      >
        <p className="text-sm">Your own GitHub credential</p>
      </ConfigurationItem>
    </ConfigurationGroup>
  )
}
