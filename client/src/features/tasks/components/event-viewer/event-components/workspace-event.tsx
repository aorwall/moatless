import type { Event } from '@shared/types/event'
import { Settings } from 'lucide-react'

import { EventHeader } from './event-header'

interface WorkspaceEventProps {
  event: Event
}

export function WorkspaceEvent({ event }: WorkspaceEventProps) {
  const payload = event.payload as {
    environmentId?: string
    path?: string
    branch?: string
    workingDirectory?: string
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <Settings className="h-4 w-4 text-muted-foreground" />
          <span className="text-sm font-medium text-foreground">
            Workspace {event.subType?.replace(/_/g, ' ')}
          </span>
        </div>

        <div className="text-sm text-muted-foreground">
          {event.subType === 'worktree_created' && (
            <div className="space-y-1">
              <p>
                Path: <code className="text-xs bg-muted px-1 py-0.5 rounded">{payload.path}</code>
              </p>
              <p>
                Branch:{' '}
                <code className="text-xs bg-muted px-1 py-0.5 rounded">{payload.branch}</code>
              </p>
            </div>
          )}
          {event.subType === 'setup_completed' && (
            <p>
              Working directory:{' '}
              <code className="text-xs bg-muted px-1 py-0.5 rounded">
                {payload.workingDirectory}
              </code>
            </p>
          )}
        </div>
      </div>
    </EventHeader>
  )
}
