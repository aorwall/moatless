import { useId } from 'react'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Label } from '@/lib/components/ui/label'

interface GitStatusConditionsProps {
  conditions?: {
    requiresUncommittedChanges?: boolean
    requiresCommittedChanges?: boolean
    requiresBehindMain?: boolean
    requiresAheadOfMain?: boolean
    requiresDiverged?: boolean
  }
  onConditionsChange: (conditions: {
    requiresUncommittedChanges?: boolean
    requiresCommittedChanges?: boolean
    requiresBehindMain?: boolean
    requiresAheadOfMain?: boolean
    requiresDiverged?: boolean
  }) => void
}

export function GitStatusConditions({ conditions, onConditionsChange }: GitStatusConditionsProps) {
  const requiresUncommittedChangesId = useId()
  const requiresCommittedChangesId = useId()
  const requiresBehindMainId = useId()
  const requiresAheadOfMainId = useId()
  const requiresDivergedId = useId()

  return (
    <div>
      <Label className="text-base font-medium mb-3 block">Git Status Conditions</Label>
      <div className="space-y-3">
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresUncommittedChangesId}
            checked={conditions?.requiresUncommittedChanges}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresUncommittedChanges: checked as boolean,
              })
            }
          />
          <Label
            htmlFor={requiresUncommittedChangesId}
            className="text-sm font-normal cursor-pointer"
          >
            Requires uncommitted changes
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresCommittedChangesId}
            checked={conditions?.requiresCommittedChanges}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresCommittedChanges: checked as boolean,
              })
            }
          />
          <Label
            htmlFor={requiresCommittedChangesId}
            className="text-sm font-normal cursor-pointer"
          >
            Requires committed changes
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresBehindMainId}
            checked={conditions?.requiresBehindMain}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresBehindMain: checked as boolean,
              })
            }
          />
          <Label htmlFor={requiresBehindMainId} className="text-sm font-normal cursor-pointer">
            Requires branch behind main
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresAheadOfMainId}
            checked={conditions?.requiresAheadOfMain}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresAheadOfMain: checked as boolean,
              })
            }
          />
          <Label htmlFor={requiresAheadOfMainId} className="text-sm font-normal cursor-pointer">
            Requires branch ahead of main
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresDivergedId}
            checked={conditions?.requiresDiverged}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresDiverged: checked as boolean,
              })
            }
          />
          <Label htmlFor={requiresDivergedId} className="text-sm font-normal cursor-pointer">
            Requires branch diverged from main
          </Label>
        </div>
      </div>
    </div>
  )
}
