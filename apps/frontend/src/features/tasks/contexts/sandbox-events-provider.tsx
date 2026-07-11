import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useTaskEventSubscription } from '@/features/events/contexts/global-event-stream-provider'
import { useHistoricalSandboxEvents } from '@/features/preview/hooks/use-task-events'
import type { SandboxEvent, StartupEvent } from '@/features/preview/types'
import type { Event } from '@/lib/types/events'
import { useTaskId } from './task-data-provider'

/** Coerce a payload value to a string only when present (null/undefined → undefined). */
function optString(value: unknown): string | undefined {
  return value != null ? String(value) : undefined
}

/** Map a raw Event to a SandboxEvent for the Events tab. */
function toSandboxEvent(event: Event): SandboxEvent | null {
  const rawSubType = (event as { subType?: string }).subType
  // taskTurn events are turn-level facts; only `turn_interrupted` belongs in the
  // sandbox/server lifecycle log — the rest (active/completed/…) are filtered out.
  const isTurnInterrupted = event.type === 'taskTurn' && rawSubType === 'turn_interrupted'
  if (event.type !== 'sandbox' && event.type !== 'server' && !isTurnInterrupted) {
    return null
  }
  const payload = (event as { payload?: Record<string, unknown> }).payload ?? {}
  const exitCodeRaw = payload['exitCode']
  return {
    eventId: event.eventId,
    type: event.type as 'sandbox' | 'server' | 'taskTurn',
    subType: rawSubType ?? '',
    timestamp: event.timestamp ? new Date(event.timestamp).toISOString() : new Date().toISOString(),
    isError: event.isError,
    message: optString(payload['message']),
    reason: optString(payload['reason']),
    serverName:
      event.type === 'server' && payload['serverName'] != null
        ? String(payload['serverName'])
        : undefined,
    nodeName: optString(payload['nodeName']),
    disruptionReason: optString(payload['disruptionReason']),
    initiator: optString(payload['initiator']),
    exitCode: typeof exitCodeRaw === 'number' ? exitCodeRaw : undefined,
    terminatedReason: optString(payload['terminatedReason']),
    pendingSeconds:
      typeof payload['pendingSeconds'] === 'number' ? payload['pendingSeconds'] : undefined,
    payload,
  }
}

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
  sandboxEvents: SandboxEvent[]
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
  const [sandboxEvents, setSandboxEvents] = useState<SandboxEvent[]>([])

  // Track seen event IDs for deduplication between historical and live SSE events
  const seenEventIds = useRef<Set<string>>(new Set())

  // Fetch historical sandbox/server events so events persist across page reloads
  const historicalEventsQuery = useHistoricalSandboxEvents(taskId)

  // Seed sandboxEvents from historical data when query resolves
  useEffect(() => {
    const data = historicalEventsQuery.data
    if (!data || data.length === 0) {
      return
    }

    const mapped: SandboxEvent[] = []
    for (const event of data) {
      if (seenEventIds.current.has(event.eventId)) {
        continue
      }
      seenEventIds.current.add(event.eventId)
      const se = toSandboxEvent(event)
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

  const handleTaskEvent = useCallback((event: Event) => {
    // Append sandbox/server events for the Events tab (with dedup)
    const se = toSandboxEvent(event)
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
  }, [])

  useTaskEventSubscription(taskId, handleTaskEvent)

  const value = useMemo<SandboxEventsContextValue>(
    () => ({ sandboxEvents, startupEvents }),
    [sandboxEvents, startupEvents],
  )

  return <SandboxEventsContext.Provider value={value}>{children}</SandboxEventsContext.Provider>
}

function useSandboxEventsContext(): SandboxEventsContextValue {
  const context = useContext(SandboxEventsContext)
  if (!context) {
    throw new Error('useSandboxEvents/useStartupEvents must be used within SandboxEventsProvider')
  }
  return context
}

/** Sandbox/server lifecycle events for the Events tab and recent-events box. */
export function useSandboxEvents(): SandboxEvent[] {
  return useSandboxEventsContext().sandboxEvents
}

/** K8s startup events for the initializing overlay. */
export function useStartupEvents(): StartupEvent[] {
  return useSandboxEventsContext().startupEvents
}
