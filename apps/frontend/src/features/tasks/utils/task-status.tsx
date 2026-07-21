import {
  AlertCircle,
  Box,
  CheckCircle,
  CircleDot,
  GitMerge,
  Hourglass,
  Loader2,
  PlayCircle,
} from 'lucide-react'
import type { ReactElement } from 'react'

export interface TaskStatusConfig {
  icon: ReactElement
  tooltip: string
  color: string
}

/**
 * Live activity of a task derived from the event stream, orthogonal to the
 * lifecycle `status` (open/closed/error). Drives the sidebar "Running" section:
 * - `agent-working`   — the agent is actively producing output
 * - `starting`        — the sandbox is initializing (spinning up)
 * - `sandbox-running` — the sandbox is ready but the agent is idle
 * - `idle`            — no live activity; task sits in the normal list
 */
export type TaskActivityState = 'agent-working' | 'starting' | 'sandbox-running' | 'idle'

/**
 * Icon/color for a task's live activity state, or `null` when idle (no
 * indicator override — fall back to the lifecycle status icon). Each active
 * state uses a distinct icon so the sidebar can tell them apart at a glance.
 */
export function getTaskActivityConfig(state: TaskActivityState): TaskStatusConfig | null {
  switch (state) {
    case 'agent-working':
      return {
        icon: <Loader2 className="h-3 w-3 animate-spin" />,
        tooltip: 'Agent working',
        color: 'text-status-success',
      }
    case 'starting':
      return {
        icon: <Hourglass className="h-3 w-3 animate-pulse" />,
        tooltip: 'Starting up',
        color: 'text-status-warning',
      }
    case 'sandbox-running':
      return {
        icon: <Box className="h-3 w-3" />,
        tooltip: 'Sandbox running',
        color: 'text-status-info',
      }
    default:
      return null
  }
}

/**
 * Determines if a task is currently running based on task status.
 * Note: The backend task status only tracks lifecycle (open/closed/error).
 * Agent running state is tracked via workspace agentState in real-time.
 */
export function isTaskRunning(task: { status: string }): boolean {
  return task.status === 'running'
}

export function getTaskStatusConfig(status: string): TaskStatusConfig {
  switch (status) {
    case 'open':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-status-info',
      }
    case 'running':
      return {
        icon: <Loader2 className="h-3 w-3 animate-spin" />,
        tooltip: 'Running',
        color: 'text-status-warning',
      }
    case 'error':
      return {
        icon: <AlertCircle className="h-3 w-3" />,
        tooltip: 'Error',
        color: 'text-status-error',
      }
    case 'merged':
      return {
        icon: <GitMerge className="h-3 w-3" />,
        tooltip: 'Merged',
        color: 'text-status-success',
      }
    case 'closed':
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Closed',
        color: 'text-status-pending',
      }
    // Legacy statuses for backward compatibility
    case 'active':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-status-info',
      }
    case 'completed':
      return {
        icon: <CheckCircle className="h-3 w-3" />,
        tooltip: 'Completed',
        color: 'text-status-success',
      }
    default:
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Unknown',
        color: 'text-status-pending',
      }
  }
}
