import { Settings } from 'lucide-react'

import type { Event } from '@/types/event'

import { EventHeader } from './event-header'

interface WorkspaceEventProps {
  event: Event
}

export function WorkspaceEvent({ event }: WorkspaceEventProps) {
  const payload = event.payload as {
    environment_id?: string
    path?: string
    branch?: string
    working_directory?: string
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <Settings className="h-4 w-4 text-muted-foreground" />
          <span className="text-sm font-medium text-foreground">
            Workspace {event.sub_type?.replace(/_/g, ' ')}
          </span>
        </div>

        <div className="text-sm text-muted-foreground">
          {event.sub_type === 'worktree_created' && (
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
          {event.sub_type === 'setup_completed' && (
            <p>
              Working directory:{' '}
              <code className="text-xs bg-muted px-1 py-0.5 rounded">
                {payload.working_directory}
              </code>
            </p>
          )}
        </div>
      </div>
    </EventHeader>
  )
}
