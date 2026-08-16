/**
 * Chain: Shared Warm Sandbox Members
 *
 * A running warm sandbox admits sibling tasks. Shared members use the lead's
 * materialized checkout; isolated members get their own checkout under the same
 * sandbox identity.
 */

import { findNamedFileWithContent } from '../../shared/content-helpers'
import { budget, missingCapability } from '../../shared/profile'
import type { TaskResponse } from '../../shared/types'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithoutMessage, waitForAgentTurnComplete, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('agentCredentials')

interface SharedWarmTaskFields {
  sandboxId?: string | null
  checkoutMode?: string | null
}

function sandboxId(task: TaskResponse): string {
  const id = (task as TaskResponse & SharedWarmTaskFields).sandboxId
  expect(id, 'task response must include the sandbox identity').toBeTruthy()
  return id!
}

function checkoutMode(task: TaskResponse): string | null | undefined {
  return (task as TaskResponse & SharedWarmTaskFields).checkoutMode
}

test.describe('Chain: Shared Warm Sandbox Members', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(agentReason !== null, agentReason ?? '')

  const events = chainEvents()
  const ctx: {
    repositoryId?: string
    sandboxId?: string
    leadTaskId?: string
    sharedTaskId?: string
    isolatedTaskId?: string
  } = {}

  const leadFile = 'warm-lead-marker.txt'
  const leadContent = 'lead-visible-from-shared-member'
  const sharedFile = 'warm-shared-member-marker.txt'
  const sharedContent = 'shared-visible-from-lead'
  const isolatedFile = 'warm-isolated-member-marker.txt'
  const isolatedContent = 'isolated-visible-only-to-member'

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.isolatedTaskId)
    await discardTask(api, ctx.sharedTaskId)
    await discardTask(api, ctx.leadTaskId)
  })

  test('TASK-01: Lead task provisions the warm sandbox', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { taskId } = await createTaskWithoutMessage(api, ctx.repositoryId!)
    ctx.leadTaskId = taskId

    const { ready, status } = await waitForSandboxReady(api, taskId)
    expect(ready, 'lead sandbox never became ready').toBe(true)
    expect(status?.sandboxStatus, 'lead sandbox status should be ready').toBe('ready')

    const lead = await api.getTask(taskId)
    ctx.sandboxId = sandboxId(lead)

    await api.writeFile(taskId, leadFile, leadContent)
    progress(`Lead task ready in sandbox ${ctx.sandboxId}`)
  })

  test('TASK-02: Shared member joins the existing checkout', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady', 'agentIdle'))

    const task = await api.createVibeTask({
      templateId: ctx.repositoryId!,
      name: 'shared warm sandbox member',
      message: `Create a file called ${sharedFile} containing exactly '${sharedContent}'.`,
      sameSandboxTaskId: ctx.leadTaskId!,
      checkoutMode: 'shared',
    })
    ctx.sharedTaskId = task.taskId

    const joined = await api.getTask(task.taskId)
    expect(sandboxId(joined), 'shared member should reuse the lead sandbox identity').toBe(
      ctx.sandboxId,
    )
    expect(checkoutMode(joined), 'shared member should persist shared checkout mode').toBe('shared')

    const { ready } = await waitForSandboxReady(api, task.taskId)
    expect(ready, 'shared member sandbox status never became ready').toBe(true)

    const { completed } = await waitForAgentTurnComplete(events(), api, task.taskId)
    expect(completed, 'shared member agent turn did not complete').toBe(true)
    progress(`Shared member joined sandbox ${ctx.sandboxId}`)
  })

  test('TASK-03: Isolated member joins with a separate checkout', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady', 'agentIdle'))

    const task = await api.createVibeTask({
      templateId: ctx.repositoryId!,
      name: 'isolated warm sandbox member',
      message: `Create a file called ${isolatedFile} containing exactly '${isolatedContent}'.`,
      sameSandboxTaskId: ctx.leadTaskId!,
      checkoutMode: 'isolated',
    })
    ctx.isolatedTaskId = task.taskId

    const joined = await api.getTask(task.taskId)
    expect(sandboxId(joined), 'isolated member should reuse the lead sandbox identity').toBe(
      ctx.sandboxId,
    )
    expect(checkoutMode(joined), 'isolated member should persist isolated checkout mode').toBe(
      'isolated',
    )

    const { ready } = await waitForSandboxReady(api, task.taskId)
    expect(ready, 'isolated member sandbox status never became ready').toBe(true)

    const { completed } = await waitForAgentTurnComplete(events(), api, task.taskId)
    expect(completed, 'isolated member agent turn did not complete').toBe(true)
    progress(`Isolated member joined sandbox ${ctx.sandboxId}`)
  })

  test('FILE-01: Shared and isolated checkout visibility is task-scoped', async ({
    api,
  }, testInfo) => {
    testInfo.setTimeout(budget())

    expect(
      await findNamedFileWithContent(api, ctx.sharedTaskId!, leadFile, leadContent),
      'shared member should see the lead checkout file',
    ).not.toBeNull()
    expect(
      await findNamedFileWithContent(api, ctx.leadTaskId!, sharedFile, sharedContent),
      'lead should see the shared member checkout file',
    ).not.toBeNull()

    expect(
      await findNamedFileWithContent(api, ctx.isolatedTaskId!, isolatedFile, isolatedContent),
      'isolated member should see its own checkout file',
    ).not.toBeNull()
    expect(
      await findNamedFileWithContent(api, ctx.leadTaskId!, isolatedFile, isolatedContent),
      'lead should not see the isolated member checkout file',
    ).toBeNull()
    expect(
      await findNamedFileWithContent(api, ctx.isolatedTaskId!, leadFile, leadContent),
      'isolated member should not see the lead checkout file',
    ).toBeNull()
    expect(
      await findNamedFileWithContent(api, ctx.isolatedTaskId!, sharedFile, sharedContent),
      'isolated member should not see the shared member checkout file',
    ).toBeNull()
  })
})
