import type { LiveStatus, SandboxStatus } from '@moatless/api-client'
import { describe, expect, it } from 'vitest'
import {
  canExecuteInSandbox,
  isSandboxReady,
  liveStatusToActivityState,
  sandboxPresence,
} from '../live-status'

function status(sandboxStatus: SandboxStatus, agentStatus?: LiveStatus['agentStatus']): LiveStatus {
  return {
    sandboxStatus,
    agentStatus: agentStatus ?? null,
    sandboxError: null,
  }
}

describe('live status derivations', () => {
  it('derives readiness only from ready sandbox status', () => {
    expect(isSandboxReady(status('ready'))).toBe(true)
    expect(isSandboxReady(status('initializing'))).toBe(false)
    expect(isSandboxReady(undefined)).toBe(false)
  })

  it('allows execution only for an idle Agent in a ready Sandbox', () => {
    expect(canExecuteInSandbox(status('ready', 'idle'))).toBe(true)
    expect(canExecuteInSandbox(status('ready', 'running'))).toBe(false)
    expect(canExecuteInSandbox(status('ready', 'unknown'))).toBe(false)
    expect(canExecuteInSandbox(status('stopped'))).toBe(false)
  })

  it('derives the diagnostics presence badge from sandbox status', () => {
    expect(sandboxPresence('not_created')).toBe('missing')
    expect(sandboxPresence('removed')).toBe('missing')
    expect(sandboxPresence('initializing')).toBe('exists')
    expect(sandboxPresence('ready')).toBe('exists')
    expect(sandboxPresence('stopped')).toBe('exists')
  })

  it('maps the trio to task-list activity state', () => {
    expect(liveStatusToActivityState(status('ready', 'running'))).toBe('agent-working')
    expect(liveStatusToActivityState(status('initializing'))).toBe('starting')
    expect(liveStatusToActivityState(status('ready', 'idle'))).toBe('sandbox-running')
    expect(liveStatusToActivityState(status('stopped'))).toBe('idle')
  })
})
