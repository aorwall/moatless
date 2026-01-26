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
    const snapshot = await coordinator.getSnapshot(taskId)
    const target = snapshot?.executionTarget as {
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
 * Wait for preview server to be ready
 */
export async function waitForPreviewReady(
  coordinator: WorkspaceRuntimeManager,
  taskId: string,
  timeoutMs: number,
): Promise<boolean> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    const snapshot = await coordinator.getSnapshot(taskId)

    if (snapshot?.preview.status === 'started') {
      return true
    }

    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  return false
}

/**
 * Wait for agent to become idle after running.
 *
 * The function first waits for the agent to start (state becomes 'running'),
 * then waits for it to become idle (state becomes 'idle').
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
    const snapshot = await coordinator.getSnapshot(taskId)
    const agentState = snapshot?.agent?.state

    // Track if we've seen the agent running
    if (agentState === 'running') {
      hasSeenRunning = true
    }

    // Only consider idle if we've seen running first
    if (hasSeenRunning && agentState === 'idle') {
      return true
    }

    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  return false
}
