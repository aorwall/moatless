/**
 * Sandbox step functions (SANDBOX-01 through SANDBOX-06).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type { MoatApi } from '../../shared/moat-api'
import { profile } from '../../shared/profile'
import type { SandboxStatusFullResponse } from '../../shared/types'
import { waitForWorkspaceReady, waitForWorkspaceStatus } from '../../shared/wait-helpers'

/**
 * How long a sandbox has to make each transition.
 *
 * Read from the profile rather than fixed: a pod that has to schedule, pull and
 * mount takes several times as long as a container that is already local, and a
 * ceiling set for the fast environment fails the slow one for no reason.
 */
function sandboxBudget(): number {
  return profile().timeouts.sandboxReady
}

function teardownBudget(): number {
  return profile().timeouts.sandboxTeardown
}

// =============================================================================
// SANDBOX-01: Wait for sandbox ready
// =============================================================================

export async function waitForSandboxReady(
  api: MoatApi,
  taskId: string,
  timeoutMs = sandboxBudget(),
): Promise<{ ready: boolean; status: SandboxStatusFullResponse | null }> {
  const ready = await waitForWorkspaceReady(api, taskId, { timeout: timeoutMs })
  if (!ready) {
    return { ready: false, status: null }
  }

  // Stability check: wait briefly and re-verify to catch transient ready
  // states during crash loops (sandbox.ready → sandbox.error cycling)
  await new Promise((resolve) => setTimeout(resolve, 3000))
  const status = await api.getWorkspaceStatus(taskId)

  if (status.sandboxStatus !== 'ready') {
    console.log(
      `[waitForSandboxReady] Sandbox was briefly ready but reverted to ${status.sandboxStatus}`,
    )
    if (status.sandboxError) {
      console.log(`[waitForSandboxReady] Error: ${status.sandboxError}`)
    }
    return { ready: false, status }
  }

  return { ready: true, status }
}

// =============================================================================
// SANDBOX-02: Idempotent re-provision
// =============================================================================

export async function provisionSandbox(
  api: MoatApi,
  taskId: string,
): Promise<{ result: unknown; status: SandboxStatusFullResponse }> {
  const result = await api.provisionSandbox(taskId)
  const status = await api.getWorkspaceStatus(taskId)
  return { result, status }
}

// =============================================================================
// SANDBOX-03: Stop sandbox
// =============================================================================

export async function stopSandbox(
  api: MoatApi,
  taskId: string,
  waitTimeoutMs = teardownBudget(),
): Promise<SandboxStatusFullResponse | null> {
  await api.stopSandbox(taskId)
  return waitForWorkspaceStatus(api, taskId, 'stopped', { timeout: waitTimeoutMs })
}

// =============================================================================
// SANDBOX-03.1: Start stopped sandbox
// =============================================================================

export async function startSandbox(
  api: MoatApi,
  taskId: string,
  waitTimeoutMs = sandboxBudget(),
): Promise<{ ready: boolean; status: SandboxStatusFullResponse | null }> {
  await api.startSandbox(taskId)
  const ready = await waitForWorkspaceReady(api, taskId, { timeout: waitTimeoutMs })

  let status: SandboxStatusFullResponse | null = null
  if (ready) {
    status = await api.getWorkspaceStatus(taskId)
  }

  return { ready, status }
}

// =============================================================================
// SANDBOX-04: Restart sandbox
// =============================================================================

export async function restartSandbox(
  api: MoatApi,
  taskId: string,
  waitTimeoutMs = sandboxBudget(),
): Promise<{ ready: boolean; status: SandboxStatusFullResponse | null }> {
  await api.restartSandbox(taskId)
  const ready = await waitForWorkspaceReady(api, taskId, { timeout: waitTimeoutMs })

  let status: SandboxStatusFullResponse | null = null
  if (ready) {
    status = await api.getWorkspaceStatus(taskId)
  }

  return { ready, status }
}

// =============================================================================
// SANDBOX-05: Cleanup sandbox
// =============================================================================

export async function cleanupSandbox(
  api: MoatApi,
  taskId: string,
  waitTimeoutMs = teardownBudget(),
): Promise<SandboxStatusFullResponse | null> {
  await api.cleanupSandbox(taskId)
  return waitForWorkspaceStatus(api, taskId, 'not_created', { timeout: waitTimeoutMs })
}
