/**
 * UI-facing lifecycle event: a normalized event shown in the Events tab and
 * recent-events box. Covers every persisted event type (agent, task, taskTurn,
 * sandbox, server) — the live-only UI streams (resources/navigation/preview)
 * are excluded by `toLifecycleEvent`. Distinct from the raw stream events in
 * `@/lib/types/events` — this is the aggregated shape the UI renders, derived
 * from the raw stream via `toLifecycleEvent`.
 */
export interface LifecycleEvent {
  eventId: string
  type: 'sandbox' | 'server' | 'taskTurn' | 'agent' | 'task'
  subType: string
  timestamp: string
  isError: boolean
  message?: string | undefined
  reason?: string | undefined
  /** Server name for server events (e.g., "preview", "files") */
  serverName?: string | undefined
  /** Agent runtime for agent events (e.g., "claude-code-cli"); used as the source label. */
  agentType?: string | undefined
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
  /**
   * Sandbox lifecycle vocabulary key (`pod_evicted`, `node_reclaimed`, …), the
   * only field the UI reads out of the raw event payload. Deliberately narrow:
   * carrying the whole payload here kept every `agent` event's full SDK message
   * alive for as long as the event log was cached, which dominated the JS heap.
   */
  lifecycleEvent?: string | undefined
}

/** K8s startup event shown in the sandbox initializing overlay. */
export interface StartupEvent {
  reason: string
  message: string
  eventType: string
  timestamp: string
}
