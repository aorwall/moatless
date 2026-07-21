import { createContext, type ReactNode, useContext, useEffect, useRef, useState } from 'react'
import type { LifecycleEvent, StartupEvent } from '@/features/events'
import {
  toLifecycleEvent,
  useHistoricalLifecycleEvents,
  useTaskEventSubscription,
} from '@/features/events'
import type { Event } from '@/lib/types/events'
import { useTaskId } from './task-data-provider'

/** Check if a sandbox event subType is a startup-phase event. */
function isStartupSubType(subType: string | undefined): boolean {
  return subType === 'initializing' || subType === 'k8s_event' || subType === 'sandbox_event'
}

/** Check if a sandbox event subType signals the end of the startup phase. */
function isStartupEndSubType(subType: string | undefined): boolean {
  return (
    subType === 'ready' || subType === 'error' || subType === 'stopped' || subType === 'removed'
  )
}

interface SandboxEventsContextValue {
  /** Sandbox/server lifecycle events for the Events tab and recent-events box. */
  sandboxEvents: LifecycleEvent[]
  /** K8s startup events for the initializing overlay. */
  startupEvents: StartupEvent[]
}

const SandboxEventsContext = createContext<SandboxEventsContextValue | undefined>(undefined)

/**
 * Accumulates sandbox/server lifecycle events (for the Events tab) and K8s
 * startup events (for the initializing overlay) from the SSE stream plus the
 * historical events query. Sits above the task-page layouts so events survive a
 * mobile↔desktop layout swap. One provider owns the single SSE subscription.
 */
export function SandboxEventsProvider({ children }: { children: ReactNode }) {
  const taskId = useTaskId()

  // K8s startup events for the initializing overlay
  const [startupEvents, setStartupEvents] = useState<StartupEvent[]>([])

  // Sandbox/server lifecycle events for Events tab and recent events box
  const [sandboxEvents, setSandboxEvents] = useState<LifecycleEvent[]>([])

  // Track seen event IDs for deduplication between historical and live SSE events
  const seenEventIds = useRef<Set<string>>(new Set())

  // Fetch historical sandbox/server events so events persist across page reloads
  const historicalEventsQuery = useHistoricalLifecycleEvents(taskId)

  // Seed sandboxEvents from historical data when query resolves
  useEffect(() => {
    const data = historicalEventsQuery.data
    if (!data || data.length === 0) {
      return
    }

    const mapped: LifecycleEvent[] = []
    for (const event of data) {
      if (seenEventIds.current.has(event.eventId)) {
        continue
      }
      seenEventIds.current.add(event.eventId)
      const se = toLifecycleEvent(event)
      if (se) {
        mapped.push(se)
      }
    }

    if (mapped.length > 0) {
      setSandboxEvents((prev) => {
        // Merge: historical first, then any live events that arrived before the query resolved
        const liveOnly = prev.filter((e) => !seenEventIds.current.has(e.eventId))
        return [...mapped, ...liveOnly]
      })
    }
  }, [historicalEventsQuery.data])

  const handleTaskEvent = (event: Event) => {
    // Append sandbox/server events for the Events tab (with dedup)
    const se = toLifecycleEvent(event)
    if (se) {
      if (seenEventIds.current.has(event.eventId)) {
        return
      }
      seenEventIds.current.add(event.eventId)
      setSandboxEvents((prev) => [...prev, se])
    }

    if (event.type === 'sandbox') {
      const rawSubType = (event as { subType?: string }).subType
      if (isStartupSubType(rawSubType)) {
        const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
        if (payload['reason'] || payload['message']) {
          setStartupEvents((prev) => [
            ...prev,
            {
              reason: String(payload['reason'] ?? ''),
              message: String(payload['message'] ?? ''),
              eventType: String(payload['eventType'] ?? 'Normal'),
              timestamp: event.timestamp
                ? new Date(event.timestamp).toISOString()
                : new Date().toISOString(),
            },
          ])
        }
      }
      if (isStartupEndSubType(rawSubType)) {
        setStartupEvents([])
      }
    }
  }

  useTaskEventSubscription(taskId, handleTaskEvent)

  const value = {
    sandboxEvents,
    startupEvents,
  }

  return <SandboxEventsContext value={value}>{children}</SandboxEventsContext>
}

function useSandboxEventsContext(): SandboxEventsContextValue {
  const context = useContext(SandboxEventsContext)
  if (!context) {
    throw new Error('useSandboxEvents/useStartupEvents must be used within SandboxEventsProvider')
  }
  return context
}

/** Sandbox/server lifecycle events for the Events tab and recent-events box. */
export function useSandboxEvents(): LifecycleEvent[] {
  return useSandboxEventsContext().sandboxEvents
}

/** K8s startup events for the initializing overlay. */
export function useStartupEvents(): StartupEvent[] {
  return useSandboxEventsContext().startupEvents
}
