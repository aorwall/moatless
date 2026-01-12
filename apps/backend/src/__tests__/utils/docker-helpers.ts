/**
 * Docker-specific test utilities
 *
 * Provides helpers for Docker workspace integration tests, including:
 * - Docker availability checks
 * - Platform compatibility verification
 * - Workspace readiness polling
 * - Preview server readiness polling
 */

import { execSync } from 'node:child_process'
import os from 'node:os'
import type { WorkspaceCoordinator } from '../../features/workspace/index.js'

/**
 * Check if Docker daemon is running and available
 */
export async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { timeout: 5000, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

/**
 * Check if the host platform is compatible with the sandbox image
 * Both x64 and arm64 architectures are supported
 */
export function isPlatformCompatible(): boolean {
  try {
    const arch = os.arch()
    return arch === 'x64' || arch === 'arm64'
  } catch {
    return false
  }
}

/**
 * Check if ANTHROPIC_API_KEY is set for agent tests
 */
export function hasAnthropicKey(): boolean {
  return !!process.env['ANTHROPIC_API_KEY']
}

/**
 * Check if Claude OAuth token is available
 */
export function hasClaudeOAuthToken(): boolean {
  return !!process.env['CLAUDE_CODE_OAUTH_TOKEN']
}

/**
 * Check if any Claude authentication is available
 */
export function hasClaudeAuth(): boolean {
  return hasAnthropicKey() || hasClaudeOAuthToken()
}

/**
 * Wait for Docker workspace to be ready
 * Polls the workspace coordinator until the container is running with a port.
 * Note: The Docker provisioner now waits for workspace provisioning (git clone)
 * to complete before returning, so we just need to wait for the snapshot.
 */
export async function waitForWorkspaceReady(
  workspaceCoordinator: WorkspaceCoordinator,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()
  const pollInterval = 1000

  while (Date.now() - startTime < timeoutMs) {
    try {
      const snapshot = await workspaceCoordinator.getSnapshot(taskId)

      if (snapshot?.executionTarget?.mode === 'docker') {
        const target = snapshot.executionTarget as {
          containerStatus?: string
          hostPort?: number
        }

        // Container is running and has a port - provisioner has completed
        // The provisioner now waits for workspace/status to be ready before returning
        if (target.containerStatus === 'running' && target.hostPort) {
          return true
        }
      }
    } catch {
      // Snapshot not available yet
    }

    await new Promise((resolve) => setTimeout(resolve, pollInterval))
  }

  return false
}

/**
 * Wait for preview server to be ready
 * Polls the workspace coordinator until preview status is 'started'
 */
export async function waitForPreviewReady(
  workspaceCoordinator: WorkspaceCoordinator,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()
  const pollInterval = 2000

  while (Date.now() - startTime < timeoutMs) {
    try {
      const snapshot = await workspaceCoordinator.getSnapshot(taskId)

      if (snapshot?.preview?.status === 'started') {
        return true
      }

      // Check if it failed
      if (snapshot?.preview?.status === 'failed') {
        console.error('Preview server failed')
        return false
      }
    } catch {
      // Snapshot not available yet
    }

    await new Promise((resolve) => setTimeout(resolve, pollInterval))
  }

  return false
}

/**
 * Wait for agent to complete
 * Polls the workspace coordinator until agent state is 'idle'
 */
export async function waitForAgentIdle(
  workspaceCoordinator: WorkspaceCoordinator,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()
  const pollInterval = 1000

  while (Date.now() - startTime < timeoutMs) {
    try {
      const snapshot = await workspaceCoordinator.getSnapshot(taskId)

      if (snapshot?.agent?.state === 'idle') {
        return true
      }

      // Agent can only be 'idle' or 'running', no error state in snapshot
      // If agent fails, it transitions back to 'idle' after completion
    } catch {
      // Snapshot not available yet
    }

    await new Promise((resolve) => setTimeout(resolve, pollInterval))
  }

  return false
}
