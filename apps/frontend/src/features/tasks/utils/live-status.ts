import type { AgentStatus, LiveStatus, SandboxStatus } from '@moatless/api-client'
import type { TaskActivityState } from './task-status'

type StatusTrio = {
  sandboxStatus: SandboxStatus
  agentStatus?: AgentStatus | null | undefined
  sandboxError?: string | null | undefined
}

export function isSandboxReady(status: Pick<LiveStatus, 'sandboxStatus'> | undefined): boolean {
  return status?.sandboxStatus === 'ready'
}

export function canExecuteInSandbox(status: StatusTrio | undefined): boolean {
  return isSandboxReady(status) && status?.agentStatus === 'idle'
}

export function sandboxPresence(status: SandboxStatus): 'exists' | 'missing' {
  return status === 'not_created' || status === 'removed' ? 'missing' : 'exists'
}

export function liveStatusToActivityState(status: StatusTrio | undefined): TaskActivityState {
  if (status?.agentStatus === 'running') {
    return 'agent-working'
  }
  if (status?.sandboxStatus === 'initializing') {
    return 'starting'
  }
  if (status?.sandboxStatus === 'ready') {
    return 'sandbox-running'
  }
  return 'idle'
}
