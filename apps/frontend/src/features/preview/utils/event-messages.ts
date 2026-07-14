import type { LucideIcon } from 'lucide-react'
import { CheckCircle2, Clock, MinusCircle, RotateCcw, Server, XCircle } from 'lucide-react'
import type { LifecycleEvent } from '@/features/events/types'

/**
 * Get the source label to display for an event.
 * Server events show the server name (e.g., "preview"); sandbox events show "sandbox".
 */
export function getSourceLabel(event: LifecycleEvent): string {
  if (event.type === 'server' && event.serverName) {
    return event.serverName
  }
  if (event.type === 'taskTurn') {
    return 'turn'
  }
  return event.type
}

/**
 * Generate a human-readable display message for an event.
 * Prefers backend-provided message, then reason, then generates a fallback
 * from the event type and subType.
 */
export function getDisplayMessage(event: LifecycleEvent): string {
  if (event.message) {
    return event.message
  }
  if (event.reason) {
    return event.reason
  }
  // Fallback: humanize the subType
  return event.subType.replace(/_/g, ' ')
}

/**
 * Visual tone for an event. Deliberately richer than error/ok so that a
 * recoverable disruption (eviction, container restart, turn interruption)
 * renders as amber "recovering" rather than a red error, and an intentional
 * stop renders neutral rather than green "success".
 */
export type EventTone = 'ok' | 'recovering' | 'error' | 'neutral'

/** A muted secondary-metadata chip rendered under the event message. */
export interface EventChip {
  label: string
  value: string
}

export interface EventDescriptor {
  /** Friendly, human-readable label (replaces the raw subType tag). */
  label: string
  icon: LucideIcon
  tone: EventTone
  /** Secondary metadata (node, cause, initiator, exit code, …). */
  chips: EventChip[]
}

/** Tailwind text-color class per tone — used for the icon and message text. */
export const TONE_TEXT_CLASS: Record<EventTone, string> = {
  ok: 'text-green-500',
  recovering: 'text-amber-500',
  error: 'text-destructive',
  neutral: 'text-muted-foreground',
}

interface Descriptor {
  label: string
  tone: EventTone
  /** Overrides the tone-derived icon (e.g. a clock for scheduling latency). */
  icon?: LucideIcon
}

/**
 * Vocabulary keyed by the sandbox `lifecycleEvent` (rich) or, for events that
 * predate it, the coarse status/server/turn `subType`. New optional subtypes
 * (`pod_scheduled`, `scheduling_blocked`, `startup_slow`, `turn_interrupted`)
 * are covered here so they render correctly the moment the backend emits them.
 */
const DESCRIPTORS: Record<string, Descriptor> = {
  // Sandbox lifecycle vocabulary (payload.lifecycleEvent).
  pod_ready: { label: 'Ready', tone: 'ok' },
  pod_recovering: { label: 'Recovering', tone: 'recovering' },
  pod_scheduled: { label: 'Scheduled', tone: 'neutral', icon: Clock },
  pod_deleted: { label: 'Deleted', tone: 'neutral' },
  pod_succeeded: { label: 'Completed', tone: 'neutral' },
  sandbox_completed: { label: 'Completed', tone: 'neutral' },
  pod_failed: { label: 'Pod failed', tone: 'error' },
  pod_evicted: { label: 'Evicted', tone: 'recovering' },
  node_reclaimed: { label: 'Node reclaimed', tone: 'recovering' },
  container_oom_killed: { label: 'Out of memory', tone: 'error' },
  container_terminated: { label: 'Terminated', tone: 'error' },
  container_crash_loop: { label: 'Crash looping', tone: 'error' },
  container_restarted: { label: 'Restarted', tone: 'recovering' },
  auto_stopped: { label: 'Auto-stopped', tone: 'neutral' },
  fatal: { label: 'Fatal', tone: 'error' },
  server_auto_disabled: { label: 'Server disabled', tone: 'error' },
  idle_reaped: { label: 'Stopped (idle)', tone: 'neutral' },
  scheduling_blocked: { label: 'Scheduling blocked', tone: 'recovering', icon: Clock },
  startup_slow: { label: 'Slow startup', tone: 'recovering', icon: Clock },
  // Coarse sandbox status (old events without a lifecycleEvent).
  ready: { label: 'Ready', tone: 'ok' },
  initializing: { label: 'Initializing', tone: 'recovering' },
  stopped: { label: 'Stopped', tone: 'neutral' },
  error: { label: 'Error', tone: 'error' },
  not_created: { label: 'Not created', tone: 'neutral' },
  // Server container status.
  starting: { label: 'Starting', tone: 'recovering' },
  installing: { label: 'Installing', tone: 'recovering' },
  started: { label: 'Started', tone: 'ok' },
  failed: { label: 'Failed', tone: 'error' },
  // Task-turn facts.
  turn_interrupted: { label: 'Turn interrupted', tone: 'recovering' },
}

/** The lifecycleEvent is where the rich sandbox vocabulary lives; fall back to subType. */
function resolveKey(event: LifecycleEvent): string {
  if (event.type === 'sandbox') {
    const lifecycle = event.payload?.['lifecycleEvent']
    if (typeof lifecycle === 'string' && lifecycle.length > 0) {
      return lifecycle
    }
  }
  return event.subType
}

function iconForTone(tone: EventTone, type: LifecycleEvent['type']): LucideIcon {
  if (tone === 'error') {
    return XCircle
  }
  if (tone === 'recovering') {
    return RotateCcw
  }
  // Server events keep their distinct server glyph when not erroring.
  if (type === 'server') {
    return Server
  }
  if (tone === 'ok') {
    return CheckCircle2
  }
  return MinusCircle
}

/** Format a whole number of seconds as a compact duration (e.g. "6m 4s"). */
export function formatDurationSeconds(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds))
  if (s < 60) {
    return `${s}s`
  }
  const minutes = Math.floor(s / 60)
  const remSeconds = s % 60
  if (minutes < 60) {
    return remSeconds > 0 ? `${minutes}m ${remSeconds}s` : `${minutes}m`
  }
  const hours = Math.floor(minutes / 60)
  const remMinutes = minutes % 60
  return remMinutes > 0 ? `${hours}h ${remMinutes}m` : `${hours}h`
}

function buildChips(event: LifecycleEvent): EventChip[] {
  const chips: EventChip[] = []
  if (event.nodeName) {
    chips.push({ label: 'node', value: event.nodeName })
  }
  if (event.disruptionReason) {
    chips.push({ label: 'cause', value: event.disruptionReason })
  }
  if (event.initiator) {
    chips.push({ label: 'by', value: event.initiator })
  }
  if (event.exitCode != null) {
    chips.push({ label: 'exit', value: String(event.exitCode) })
  }
  if (event.terminatedReason) {
    chips.push({ label: 'reason', value: event.terminatedReason })
  }
  if (event.pendingSeconds != null) {
    chips.push({ label: 'pending', value: formatDurationSeconds(event.pendingSeconds) })
  }
  return chips
}

/**
 * Central event formatter: resolves label, icon, tone, and metadata chips for a
 * single event. Used by the event log console tab.
 */
export function describeEvent(event: LifecycleEvent): EventDescriptor {
  const key = resolveKey(event)
  const descriptor = DESCRIPTORS[key]

  // Unknown subtypes: humanize the key and fall back to an error tone only when
  // the wire event is explicitly flagged as an error.
  const tone: EventTone = descriptor?.tone ?? (event.isError ? 'error' : 'neutral')
  const label = descriptor?.label ?? key.replace(/_/g, ' ')
  const icon = descriptor?.icon ?? iconForTone(tone, event.type)

  return { label, icon, tone, chips: buildChips(event) }
}
