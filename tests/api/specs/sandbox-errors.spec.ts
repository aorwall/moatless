/**
 * Standalone: Sandbox Error Cases
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Every scenario here needs a deliberately broken run configuration, which
 * lives on the workspace rather than the repository: a bad image, or a server
 * that never binds its port.
 *
 * Scenarios:
 *   SANDBOX-06: Sandbox provision failure (invalid image)
 *   SANDBOX-07: Sandbox crash loop detected early
 *   SERVER-02: Server readiness timeout detected as failure
 */

import { execSync } from 'node:child_process'
import { budget, missingCapability } from '../../shared/profile'
import { scopedName } from '../../shared/run-scope'
import { waitForPreviewReady } from '../../shared/wait-helpers'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { waitForSandboxReady } from '../steps'

const localImagesReason = missingCapability('localImages')

/** Long enough to prove a sandbox is not coming up, short enough not to sit on a known failure. */
const NOT_COMING_UP_MS = 60_000

/**
 * Ceiling for crash-loop detection. The backend gives up at
 * `max_crash_restart_count` restarts (default 5); with Docker's exponential
 * backoff those accumulate in seconds, so the test also asserts it took much
 * less than this.
 */
const CRASH_LOOP_WAIT_MS = 120_000

/** The failing server's own `startupTimeoutSeconds` is 30s, plus room to observe it. */
const SERVER_FAILURE_WAIT_MS = 120_000

// ─── SANDBOX-06: Sandbox provision failure (invalid image) ──────────────

test('SANDBOX-06: Sandbox provision failure (invalid image)', async ({ api }, testInfo) => {
  testInfo.setTimeout(NOT_COMING_UP_MS + budget())

  const repo = await api.ensureTemplateRepository()
  const workspace = await api.createWorkspace({
    name: scopedName('crash-test-imagepull'),
    dockerImage: 'nonexistent-image-12345:latest',
    repos: [{ repositoryId: repo.id }],
  })

  let taskId: string | undefined
  try {
    const task = await api.createVibeTask({ workspaceId: workspace.id })
    taskId = task.taskId

    const { ready } = await waitForSandboxReady(api, taskId, NOT_COMING_UP_MS)
    expect(ready).toBe(false)

    // In Docker mode, the provisioning error occurs at container creation time
    // (image pull failure). The status resolver sees no container → not_created.
    // The error was reported via the provision response / sandbox.error event,
    // not persisted in the status endpoint.
    const status = await api.getWorkspaceStatus(taskId)
    expect(status.sandboxStatus).not.toBe('ready')
    progress(
      `Status: ${status.sandboxStatus}, error: ${status.sandboxError ?? 'none (provision-time error)'}`,
    )
  } finally {
    if (taskId) {
      await discardTask(api, taskId)
    }
    await api.deleteWorkspace(workspace.id).catch(() => {})
  }
})

// ─── SANDBOX-07: Sandbox crash loop detected early ──────────────────────

test('SANDBOX-07: Sandbox crash loop detected early', async ({ api }, testInfo) => {
  testInfo.setTimeout(CRASH_LOOP_WAIT_MS + budget())

  // The crashing image is built here and pushed nowhere, so it can only run
  // where the runtime schedules onto this machine.
  test.skip(localImagesReason !== null, localImagesReason ?? '')

  // Build the crashing image — skip test if Docker build fails
  try {
    execSync(
      'docker build -t moatless-sandbox-crash:test -f docker/Dockerfile.sandbox-crash docker/',
      { encoding: 'utf-8', timeout: 60_000, stdio: 'pipe' },
    )
  } catch {
    test.skip(true, 'Docker build not available — skipping crash loop test')
    return
  }

  const repo = await api.ensureTemplateRepository()
  const workspace = await api.createWorkspace({
    name: scopedName('crash-test-crashloop'),
    dockerImage: 'moatless-sandbox-crash:test',
    repos: [{ repositoryId: repo.id }],
  })

  let taskId: string | undefined
  try {
    const startTime = Date.now()
    const task = await api.createVibeTask({ workspaceId: workspace.id })
    taskId = task.taskId

    // Should fail fast once the crash loop is detected.
    const { ready } = await waitForSandboxReady(api, taskId, CRASH_LOOP_WAIT_MS)
    const elapsed = Date.now() - startTime

    expect(ready).toBe(false)

    const status = await api.getWorkspaceStatus(taskId)
    // Crash loop should produce a sandboxError
    expect(status.sandboxError).toBeTruthy()
    // Should detect crash loop much faster than the full timeout
    expect(elapsed).toBeLessThan(CRASH_LOOP_WAIT_MS)

    progress(`Error: ${status.sandboxError} (detected in ${elapsed}ms)`)
  } finally {
    if (taskId) {
      await discardTask(api, taskId)
    }
    await api.deleteWorkspace(workspace.id).catch(() => {})
  }
})

// ─── SERVER-02: Server readiness timeout detected as failure ─────────

test('SERVER-02: Server readiness timeout detected as failure', async ({ api }, testInfo) => {
  testInfo.setTimeout(budget('sandboxReady') + SERVER_FAILURE_WAIT_MS)

  // A server that intentionally doesn't bind the expected port: the start
  // command runs a process that stays alive but never listens, so the
  // readiness probe stays unhealthy until the startup timeout elapses.
  const repo = await api.ensureTemplateRepository()
  const workspace = await api.createWorkspace({
    name: scopedName('server-timeout-test'),
    repos: [{ repositoryId: repo.id }],
    servers: [
      {
        name: 'preview',
        label: 'Preview',
        port: 5173,
        startCommands: ['sleep 300'], // alive but never binds port
        startupTimeoutSeconds: 30, // short timeout for faster test
        default: true,
      },
    ],
  })

  let taskId: string | undefined
  try {
    const task = await api.createVibeTask({ workspaceId: workspace.id })
    taskId = task.taskId

    // Wait for sandbox environment to be ready first
    const { ready } = await waitForSandboxReady(api, taskId)
    expect(ready).toBe(true)

    // Now wait for the server — it should eventually report "failed"
    // after the 30s startup timeout elapses
    const server = await waitForPreviewReady(api, taskId, {
      timeout: SERVER_FAILURE_WAIT_MS,
    })

    expect(server).not.toBeNull()
    expect(server!.status).toBe('failed')
    expect(server!.error).toBeTruthy()

    progress(`Server status: ${server!.status}, error: ${server!.error}`)
  } finally {
    if (taskId) {
      await discardTask(api, taskId)
    }
    await api.deleteWorkspace(workspace.id).catch(() => {})
  }
})
