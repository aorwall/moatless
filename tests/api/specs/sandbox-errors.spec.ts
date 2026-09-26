/**
 * Standalone: Sandbox Error Cases
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Every scenario here needs a deliberately broken run configuration, which
 * lives on the workspace rather than the repository: a bad image.
 *
 * Scenarios:
 *   SANDBOX-06: Sandbox provision failure (invalid image)
 *   SANDBOX-07: Sandbox crash loop detected early
 */

import { execSync } from 'node:child_process'
import { budget, missingCapability } from '../../shared/profile'
import { scopedName } from '../../shared/run-scope'
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

// ─── SANDBOX-06: Sandbox provision failure (invalid image) ──────────────

test('SANDBOX-06: Sandbox provision failure (invalid image)', async ({ api }, testInfo) => {
  const reason = missingCapability('workspaceImage')
  test.skip(reason !== null, reason ?? '')
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
    expect(status.sandboxError).toBeTruthy()
    expect(elapsed).toBeLessThan(CRASH_LOOP_WAIT_MS)

    progress(`Error: ${status.sandboxError} (detected in ${elapsed}ms)`)
  } finally {
    if (taskId) {
      await discardTask(api, taskId)
    }
    await api.deleteWorkspace(workspace.id).catch(() => {})
  }
})
