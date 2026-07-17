import type { TeamResponse } from '@/features/secrets/hooks/use-secrets'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface TeamSelectProps {
  id?: string
  value: string
  onValueChange: (value: string) => void
  teams: TeamResponse[]
  isLoading?: boolean
  error?: Error | null
  disabled?: boolean
}

export function TeamSelect({
  id,
  value,
  onValueChange,
  teams,
  isLoading = false,
  error = null,
  disabled = false,
}: TeamSelectProps) {
  const selectedTeam = teams.find((team) => team.id === value)
  const showUnknownSelection = value && !selectedTeam

  return (
    <Select value={value} onValueChange={onValueChange} disabled={disabled || isLoading || !!error}>
      <SelectTrigger id={id}>
        <SelectValue placeholder={isLoading ? 'Loading teams...' : 'Select a team'} />
      </SelectTrigger>
      <SelectContent>
        {error ? (
          <SelectItem value="__teams_error__" disabled>
            Failed to load teams
          </SelectItem>
        ) : teams.length === 0 ? (
          <SelectItem value="__no_teams__" disabled>
            {isLoading ? 'Loading teams...' : 'No teams available'}
          </SelectItem>
        ) : (
          teams.map((team) => (
            <SelectItem key={team.id} value={team.id}>
              {team.name} ({team.slug})
            </SelectItem>
          ))
        )}
        {showUnknownSelection && (
          <SelectItem value={value} disabled>
            Selected team ({value})
          </SelectItem>
        )}
      </SelectContent>
    </Select>
  )
}
