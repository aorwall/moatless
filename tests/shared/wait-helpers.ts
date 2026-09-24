/**
 * Wait/polling utilities for E2E tests.
 *
 * Provides helper functions to wait for various async conditions
 * during E2E testing.
 */

import type { SandboxStatusFullResponse } from '@moatless/api-client/sandbox/model'
import type { MoatApi } from './moat-api'
import { profile } from './profile'

// =============================================================================
// Types
// =============================================================================

export interface WaitOptions {
  /** Maximum time to wait, in milliseconds. Defaults to the profile's ceiling
   * for the transition being waited on. */
  timeout?: number
  /** Polling interval in milliseconds (default: 2000) */
  interval?: number
  /** Optional callback for progress updates */
  onProgress?: (message: string) => void
}

// =============================================================================
// Wait Functions
// =============================================================================

/**
 * Wait for workspace to become ready (environment provisioned)
 *
 * @returns true if workspace is ready, false if timeout
 */
export async function waitForWorkspaceReady(
  api: MoatApi,
  taskId: string,
  options?: WaitOptions,
): Promise<boolean> {
  const { timeout = profile().timeouts.sandboxReady, interval = 2000, onProgress } = options || {}
  const startTime = Date.now()

  while (Date.now() - startTime < timeout) {
    try {
      const status = await api.getWorkspaceStatus(taskId)

      // Fail fast if the sandbox reports a permanent error (crash loop, OOM, image pull failure)
      if (status.sandboxError) {
        onProgress?.(`Sandbox error: ${status.sandboxError}`)
        return false
      }

      if (status.sandboxStatus === 'ready') {
        onProgress?.('Workspace is ready')
        return true
      }

      onProgress?.(`Workspace status: ${status.sandboxStatus}`)
    } catch (err) {
      onProgress?.(`Error checking workspace: ${err}`)
    }

    await new Promise((resolve) => setTimeout(resolve, interval))
  }

  return false
}

/**
 * Wait for agent to complete execution (become idle after running)
 *
 * This function first waits for the agent to start (state becomes 'running'),
 * then waits for it to become idle (state becomes 'idle').
 * This prevents false positives when agent hasn't started yet.
 *
 * @returns true if agent completed, false if timeout
 */
export async function waitForAgentComplete(
  api: MoatApi,
  taskId: string,
  options?: WaitOptions,
): Promise<boolean> {
  const { timeout = profile().timeouts.agentIdle, interval = 2000, onProgress } = options || {}
  const startTime = Date.now()
  let hasSeenRunning = false

  while (Date.now() - startTime < timeout) {
    try {
      const status = await api.getWorkspaceStatus(taskId)
      const agentState = status.agentStatus

      // Track if we've seen the agent running
      if (agentState === 'running') {
        hasSeenRunning = true
        onProgress?.('Agent is running...')
        await new Promise((resolve) => setTimeout(resolve, interval))
        continue
      }

      // Agent is idle or gone — if we saw it running, it completed
      if (hasSeenRunning && (agentState === 'idle' || agentState === null)) {
        onProgress?.('Agent completed')
        return true
      }

      onProgress?.(`Agent state: ${agentState || 'unknown'}`)
    } catch (err) {
      onProgress?.(`Error checking agent: ${err}`)
    }

    await new Promise((resolve) => setTimeout(resolve, interval))
  }

  return false
}

/**
 * Wait for a specific workspace status
 *
 * @returns SandboxStatusFullResponse if matched, null if timeout
 */
export async function waitForWorkspaceStatus(
  api: MoatApi,
  taskId: string,
  expectedStatus: string,
  options?: WaitOptions,
): Promise<SandboxStatusFullResponse | null> {
  const {
    timeout = profile().timeouts.sandboxTeardown,
    interval = 2000,
    onProgress,
  } = options || {}
  const startTime = Date.now()

  while (Date.now() - startTime < timeout) {
    try {
      const status = await api.getWorkspaceStatus(taskId)

      if (status.sandboxStatus === expectedStatus) {
        onProgress?.(`Workspace status is ${expectedStatus}`)
        return status
      }

      onProgress?.(`Current status: ${status.sandboxStatus}, waiting for ${expectedStatus}`)
    } catch (err) {
      onProgress?.(`Error checking status: ${err}`)
    }

    await new Promise((resolve) => setTimeout(resolve, interval))
  }

  return null
}

/**
 * Wait for a condition to be true
 *
 * Generic polling helper for custom conditions.
 *
 * @returns true if condition was met, false if timeout
 */
export async function waitForCondition(
  condition: () => Promise<boolean>,
  options?: WaitOptions,
): Promise<boolean> {
  const { timeout = 60_000, interval = 2000, onProgress } = options || {}
  const startTime = Date.now()

  while (Date.now() - startTime < timeout) {
    try {
      if (await condition()) {
        onProgress?.('Condition met')
        return true
      }
    } catch (err) {
      onProgress?.(`Error checking condition: ${err}`)
    }

    await new Promise((resolve) => setTimeout(resolve, interval))
  }

  return false
}
