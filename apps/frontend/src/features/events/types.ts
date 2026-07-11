/**
 * UI-facing lifecycle event: a normalized sandbox/server (and turn-interrupted)
 * event shown in the Events tab and recent-events box. Distinct from the raw
 * stream `SandboxEvent` in `@/lib/types/events` — this is the aggregated shape
 * the UI renders, derived from the raw stream via `toLifecycleEvent`.
 */
export interface LifecycleEvent {
  eventId: string
  type: 'sandbox' | 'server' | 'taskTurn'
  subType: string
  timestamp: string
  isError: boolean
  message?: string | undefined
  reason?: string | undefined
  /** Server name for server events (e.g., "preview", "files") */
  serverName?: string | undefined
  /** Node the pod was scheduled on — enables node-level correlation of stops. */
  nodeName?: string | undefined
  /** Why the pod was disrupted (DisruptionTarget reason / pod status reason). */
  disruptionReason?: string | undefined
  /** Who caused an intentional stop ("reaper" | "user" | "eviction" | …). */
  initiator?: string | undefined
  /** Exit code of the terminated sandbox container, when known. */
  exitCode?: number | undefined
  /** K8s termination reason of the sandbox container, when known. */
  terminatedReason?: string | undefined
  /** Seconds spent Pending before the pod was scheduled (on pod_scheduled). */
  pendingSeconds?: number | undefined
  payload: Record<string, unknown>
}

/** K8s startup event shown in the sandbox initializing overlay. */
export interface StartupEvent {
  reason: string
  message: string
  eventType: string
  timestamp: string
}
