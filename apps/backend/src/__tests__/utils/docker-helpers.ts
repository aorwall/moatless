import { execSync } from 'node:child_process'
import os from 'node:os'
import type { WorkspaceRuntimeManager } from '../../features/workspace/index.js'

export async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { timeout: 5000, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

export function isPlatformCompatible(): boolean {
  const arch = os.arch()
  return arch === 'x64' || arch === 'arm64'
}

export function hasClaudeAuth(): boolean {
  return !!(process.env['ANTHROPIC_API_KEY'] || process.env['CLAUDE_CODE_OAUTH_TOKEN'])
}

/**
 * Wait for workspace to become ready (running state)
 */
export async function waitForWorkspaceReady(
  coordinator: WorkspaceRuntimeManager,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    const record = await coordinator.getSandboxRecord(taskId)
    const target = record?.executionTarget as {
      status?: string
      containerStatus?: string
      podPhase?: string
    }

    if (
      target?.status === 'ready' ||
      target?.containerStatus === 'running' ||
      target?.podPhase === 'Running'
    ) {
      return true
    }

    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  return false
}

/**
 * Wait for preview server to be ready.
 * Preview state is ephemeral — query the sandbox URL for live status.
 */
export async function waitForPreviewReady(
  coordinator: WorkspaceRuntimeManager,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    const sandboxUrl = await coordinator.getSandboxUrl(taskId)
    if (sandboxUrl) {
      try {
        const response = await fetch(`${sandboxUrl}/servers`, {
          signal: AbortSignal.timeout(3000),
        })
        if (response.ok) {
          const data = (await response.json()) as { servers?: Record<string, { status?: string }> }
          const servers = data.servers ?? {}
          const hasStartedServer = Object.values(servers).some((s) => s.status === 'started')
          if (hasStartedServer) {
            return true
          }
        }
      } catch {
        // Sandbox not reachable yet — retry
      }
    }

    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  return false
}

/**
 * Wait for agent to become idle after running.
 *
 * Agent state is ephemeral — queried live from the sandbox.
 * The function first waits for the agent to start (isAgentActuallyRunning returns true),
 * then waits for it to become idle (isAgentActuallyRunning returns false).
 * This prevents false positives when agent hasn't started yet.
 */
export async function waitForAgentIdle(
  coordinator: WorkspaceRuntimeManager,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()
  let hasSeenRunning = false

  while (Date.now() - startTime < timeoutMs) {
    const isRunning = await coordinator.isAgentActuallyRunning(taskId)

    // Track if we've seen the agent running
    if (isRunning) {
      hasSeenRunning = true
    }

    // Only consider idle if we've seen running first
    if (hasSeenRunning && !isRunning) {
      return true
    }

    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  return false
}
