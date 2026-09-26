/**
 * Chain: Sandbox Lifecycle
 *
 * Tests sandbox provisioning, idempotent re-provision, stop, start, restart,
 * and cleanup. Uses a task without an initial message so the agent stays idle.
 *
 * Chain:
 *   TASK-02 → SANDBOX-01 → SANDBOX-01.1 → SANDBOX-02 → SANDBOX-03 →
 *   SANDBOX-03.1 → SANDBOX-03.5 → SANDBOX-03.2 → SANDBOX-04 → SANDBOX-05 →
 *   SANDBOX-05.1
 */

import { filePaths } from '../../shared/content-helpers'
import { budget, missingCapability, profile } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import {
  cleanupSandbox,
  createTaskWithoutMessage,
  provisionSandbox,
  restartSandbox,
  startSandbox,
  stopSandbox,
  waitForSandboxReady,
} from '../steps'

/** Written before the sandbox is stopped, read back after it comes up again. */
const MARKER_PATH = 'lifecycle-marker.txt'
const MARKER_CONTENT = 'sandbox lifecycle test'

const storedWorkspaceReason = missingCapability('storedWorkspace')

test.describe('Chain: Sandbox Lifecycle', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const events = chainEvents()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
  })

  // ─── TASK-02: Create task without message ───────────────────────────

  test('TASK-02: Create task without message', async ({ api }) => {
    const { taskId, task } = await createTaskWithoutMessage(api, ctx.repositoryId!)

    expect(task.status).toBe('open')
    expect(taskId).toBeTruthy()

    ctx.taskId = taskId
    ctx.task = task

    progress(`Task created: ${taskId}`)
  })

  // ─── SANDBOX-01: Sandbox provisions (agent stays idle) ──────────────

  test('SANDBOX-01: Sandbox provisions (agent stays idle)', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('ready')

    // No environment error on healthy sandbox
    expect(status!.sandboxError).toBeFalsy()

    // Agent should be idle (no message to process)
    expect(['idle', null]).toContain(status!.agentStatus)

    ctx.sandboxStatus = status!

    // Everything downstream that claims files survive a stop or a restart
    // checks this file. Counting entries would not do: the checkout is there
    // on a freshly provisioned sandbox too, so a volume that was silently
    // recreated would still look like one that persisted.
    await api.writeFile(ctx.taskId!, MARKER_PATH, MARKER_CONTENT)

    progress(`Ready, agentStatus=${status!.agentStatus}`)
  })

  // ─── SANDBOX-01.1: SSE sandbox.ready event ─────────────────────────

  test('SANDBOX-01.1: SSE sandbox.ready event', async () => {
    const reason = missingCapability('sandboxReadyEvent')
    test.skip(reason !== null, reason ?? '')

    const event = await events().waitForEvent({
      type: 'sandbox',
      subType: 'ready',
      taskId: ctx.taskId,
      timeout: 30_000,
    })

    expect(event).not.toBeNull()
    expect(event!.taskId).toBe(ctx.taskId)
  })

  // ─── SANDBOX-02: Idempotent re-provision returns ready ──────────────

  test('SANDBOX-02: Idempotent re-provision returns ready', async ({ api }) => {
    const { status } = await provisionSandbox(api, ctx.taskId!)

    expect(status.sandboxStatus).toBe('ready')
    progress('Re-provision returned ready (idempotent)')
  })

  // ─── SANDBOX-03: Stop sandbox ───────────────────────────────────────

  test('SANDBOX-03: Stop sandbox', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxTeardown'))

    const status = await stopSandbox(api, ctx.taskId!)

    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('stopped')

    progress('Stopped')
  })

  // ─── SANDBOX-03.1: SSE sandbox.stopped event ───────────────────────

  test('SANDBOX-03.1: SSE sandbox.stopped event', async () => {
    const event = await events().waitForEvent({
      type: 'sandbox',
      subType: 'stopped',
      taskId: ctx.taskId,
      timeout: 5000,
    })
    expect(event).not.toBeNull()
  })

  // ─── SANDBOX-03.5: Read a stopped task's files from the store ──────

  test("SANDBOX-03.5: Read a stopped task's files without starting it", async ({
    api,
  }, testInfo) => {
    test.skip(storedWorkspaceReason !== null, storedWorkspaceReason ?? '')
    testInfo.setTimeout(budget('sandboxTeardown'))

    // The only read in the suite that never touches a live pod.
    //
    // `stopped` is reported once the Deployment's desired replicas reach 0, not
    // once the pod is gone, so poll: a connector that still reaches the
    // terminating pod answers `sandbox` and fails this on timing.
    let tree = await api.getFileTree(ctx.taskId!, '.', true)
    await expect
      .poll(
        async () => {
          tree = await api.getFileTree(ctx.taskId!, '.', true)
          return tree.origin.source
        },
        {
          message: 'a stopped task must answer from the store',
          timeout: profile().timeouts.sandboxTeardown,
        },
      )
      .toBe('snapshot')

    // Every assertion below is about the read the poll settled on.
    expect(tree.origin.syncedAt, 'a stored answer says when it was stored').toBeTruthy()

    const paths = filePaths(tree.tree)
    expect(paths, 'the marker file is missing from the stored tree').toContain(MARKER_PATH)

    expect(
      await api.readWorkspaceFile(ctx.taskId!, MARKER_PATH),
      'the stored copy did not answer for a file the tree lists',
    ).toBe(MARKER_CONTENT)

    progress(`Read ${paths.length} stored paths with no sandbox running`)
  })

  // ─── SANDBOX-03.2: Start stopped sandbox (files persist) ───────────

  test('SANDBOX-03.2: Start stopped sandbox (files persist)', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await startSandbox(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('ready')

    // Verify files are still present
    const files = await api.listWorkspaceFiles(ctx.taskId!, '.', false)
    expect(files.length).toBeGreaterThan(0)

    expect(
      await api.readWorkspaceFile(ctx.taskId!, MARKER_PATH),
      'the file written before the stop did not survive it',
    ).toBe(MARKER_CONTENT)

    progress(`Restarted, ${files.length} files preserved`)
  })

  // ─── SANDBOX-04: Restart sandbox (files persist) ────────────────────

  test('SANDBOX-04: Restart sandbox (files persist)', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await restartSandbox(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('ready')

    // Verify files are still present after restart
    const files = await api.listWorkspaceFiles(ctx.taskId!, '.', false)
    expect(files.length).toBeGreaterThan(0)

    expect(
      await api.readWorkspaceFile(ctx.taskId!, MARKER_PATH),
      'the file written before the restart did not survive it',
    ).toBe(MARKER_CONTENT)

    progress(`Restarted, ${files.length} files preserved`)
  })

  // ─── SANDBOX-05: Cleanup sandbox (all resources removed) ───────────

  test('SANDBOX-05: Cleanup sandbox (all resources removed)', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxTeardown'))

    const status = await cleanupSandbox(api, ctx.taskId!)

    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('not_created')

    progress('Cleaned up')
  })

  // ─── SANDBOX-05.1: SSE sandbox.removed event ────────────────────────

  test('SANDBOX-05.1: SSE sandbox.removed event', async () => {
    const event = await events().waitForEvent({
      type: 'sandbox',
      subType: 'removed',
      taskId: ctx.taskId,
      timeout: 15_000,
    })

    expect(event).not.toBeNull()
    expect(event!.taskId).toBe(ctx.taskId)
  })
})
