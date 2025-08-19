import type { Event, ServerEvent, ServerStatusEnum } from '@moatless/types'
import { useCallback, useState } from 'react'
import { useEventTypeSubscription } from '@/features/events/contexts/global-event-stream-provider'

interface UseServerEventsResult {
  serverStatus: ServerStatusEnum | null
  serverError: string | null
  serverPort: number | undefined
  serverUrl: string | undefined
  lastServerEvent: ServerEvent | null
}

// Type guard to check if event is a ServerEvent
function isServerEvent(event: Event): event is ServerEvent {
  return event.type === 'server'
}

export function useServerEvents(taskId: string | undefined): UseServerEventsResult {
  const [serverStatus, setServerStatus] = useState<ServerStatusEnum | null>(null)
  const [serverError, setServerError] = useState<string | null>(null)
  const [serverPort, setServerPort] = useState<number | undefined>(undefined)
  const [serverUrl, setServerUrl] = useState<string | undefined>(undefined)
  const [lastServerEvent, setLastServerEvent] = useState<ServerEvent | null>(null)

  const handleServerEvent = useCallback(
    (event: Event) => {
      // Only handle server events for the specified task
      if (!taskId || event.taskId !== taskId || !isServerEvent(event)) {
        return
      }

      console.log('[useServerEvents] Received server event:', {
        status: event.payload.status,
        previousStatus: event.payload.previousStatus,
        port: event.payload.port,
        url: event.payload.url,
        error: event.payload.error,
      })

      setLastServerEvent(event)
      // Map old status values to new ones
      const mappedStatus =
        event.payload.status === 'running'
          ? 'started'
          : event.payload.status === 'initializing'
            ? 'starting'
            : event.payload.status === 'failed'
              ? 'failed'
              : 'stopped'
      setServerStatus(mappedStatus)
      setServerError(event.payload.error || null)
      setServerPort(event.payload.port)
      setServerUrl(event.payload.url)
    },
    [taskId],
  )

  // Subscribe to server events
  useEventTypeSubscription('server', handleServerEvent)

  return {
    serverStatus,
    serverError,
    serverPort,
    serverUrl,
    lastServerEvent,
  }
}
