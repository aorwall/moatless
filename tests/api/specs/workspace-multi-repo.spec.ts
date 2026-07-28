/**
 * Chain: Multi-repo workspace (specs/workspaces/02-multi-repo-scenarios.md)
 *
 * A workspace with several repository placements checks out every one of them
 * into the task's sandbox, each under its own mount name.
 *
 * Both placements point at the same repository under different mount names.
 * That is a legitimate multi-repo workspace — the same repository placed twice
 * is how you get two branches side by side — and it exercises the path under
 * test, two checkouts and one primary, without needing a second fixture
 * repository to be reachable.
 *
 * Chain: WS-30 → WS-30.1 → WS-30.2
 */

import { randomUUID } from 'node:crypto'
import { budget } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { discardTask } from '../helpers/teardown'
import { waitForSandboxReady } from '../steps'

const PRIMARY_MOUNT = 'app-primary'
const SECONDARY_MOUNT = 'app-secondary'

test.describe('Chain: Multi-repo workspace', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
    const workspaceId = ctx.get<string>('workspaceId')
    if (workspaceId) {
      await api.deleteWorkspace(workspaceId).catch(() => {})
    }
  })

  // ─── WS-30: A workspace holds several repository placements ──────────

  test('WS-30: A workspace holds several repository placements', async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    const workspace = await api.createWorkspace({
      name: `multi-repo-${randomUUID()}`,
      repos: [
        {
          repositoryId: repo.id,
          mountName: PRIMARY_MOUNT,
          isPrimary: true,
          position: 0,
        },
        {
          repositoryId: repo.id,
          mountName: SECONDARY_MOUNT,
          isPrimary: false,
          position: 1,
        },
      ],
    })

    expect(workspace.repos).toHaveLength(2)
    // `kind` is derived from the placement count, so two makes it multi.
    expect(workspace.kind).toBe('multi')

    ctx.set('workspaceId', workspace.id)
  })

  // ─── WS-30.1: Every placement is checked out under its mount name ────

  test('WS-30.1: Every placement is checked out under its mount name', async ({
    api,
  }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const task = await api.createVibeTask({
      workspaceId: ctx.get<string>('workspaceId')!,
      name: 'Multi-repo workspace E2E',
    })
    ctx.taskId = task.taskId

    const { ready } = await waitForSandboxReady(api, task.taskId)
    expect(ready).toBe(true)

    // The file browser is rooted at the workspace directory, so each
    // placement shows up as its own top-level directory.
    const files = await api.listWorkspaceFiles(task.taskId)
    for (const mount of [PRIMARY_MOUNT, SECONDARY_MOUNT]) {
      expect(files.map((path) => path.replace(/\/$/, ''))).toContain(mount)
    }
  })

  // ─── WS-30.2: Each mount is a real checkout, not an empty directory ──

  test('WS-30.2: Each mount is a real checkout, not an empty directory', async ({ api }) => {
    for (const mount of [PRIMARY_MOUNT, SECONDARY_MOUNT]) {
      const content = await api.readWorkspaceFile(ctx.taskId!, `${mount}/package.json`)
      expect(content, `${mount} should be a checkout of the repository`).toBeTruthy()
    }
  })
})
