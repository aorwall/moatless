import { useId } from 'react'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Label } from '@/lib/components/ui/label'

interface TaskStatusConditionsProps {
  conditions?: {
    requiresTaskNotRunning?: boolean
    requiresLastIterationError?: boolean
    requiresLastIterationSuccess?: boolean
  }
  onConditionsChange: (conditions: {
    requiresTaskNotRunning?: boolean
    requiresLastIterationError?: boolean
    requiresLastIterationSuccess?: boolean
  }) => void
}

export function TaskStatusConditions({
  conditions,
  onConditionsChange,
}: TaskStatusConditionsProps) {
  const requiresTaskNotRunningId = useId()
  const requiresLastIterationErrorId = useId()
  const requiresLastIterationSuccessId = useId()

  return (
    <div>
      <Label className="text-base font-medium mb-3 block">Task Status Conditions</Label>
      <div className="space-y-3">
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresTaskNotRunningId}
            checked={conditions?.requiresTaskNotRunning ?? false}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresTaskNotRunning: checked as boolean,
              })
            }
          />
          <Label htmlFor={requiresTaskNotRunningId} className="text-sm font-normal cursor-pointer">
            Requires task not running
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresLastIterationErrorId}
            checked={conditions?.requiresLastIterationError ?? false}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresLastIterationError: checked as boolean,
              })
            }
          />
          <Label
            htmlFor={requiresLastIterationErrorId}
            className="text-sm font-normal cursor-pointer"
          >
            Requires last iteration had error
          </Label>
        </div>
        <div className="flex items-center space-x-2">
          <Checkbox
            id={requiresLastIterationSuccessId}
            checked={conditions?.requiresLastIterationSuccess ?? false}
            onCheckedChange={(checked) =>
              onConditionsChange({
                ...conditions,
                requiresLastIterationSuccess: checked as boolean,
              })
            }
          />
          <Label
            htmlFor={requiresLastIterationSuccessId}
            className="text-sm font-normal cursor-pointer"
          >
            Requires last iteration was successful
          </Label>
        </div>
      </div>
    </div>
  )
}
