/**
 * Chain: Workspace git override (specs/workspaces/01-git-override-scenarios.md)
 *
 * A workspace a repository declares is read-only: the repository is the source
 * of truth, and editing it in the UI would be edited away by the next sync.
 * Overriding takes manual ownership without changing how the workspace runs,
 * and keeps the origin, so a reset back to git stays possible.
 *
 * The reclaim paths (WS-06, WS-07) need a reachable repository that really
 * declares the workspace — the sync reads the config over the provider API —
 * so they are not covered here.
 *
 * Chain: WS-20 → WS-01 → WS-02 → WS-03 → WS-04 → WS-21 → WS-08
 */

import { randomUUID } from 'node:crypto'
import { missingCapability } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { psql } from '../helpers/psql'

test.describe('Chain: Workspace git override', () => {
  // Standing in for a sync means writing the workspace's provenance directly:
  // nothing in the API will claim a workspace came from git.
  const reason = missingCapability('directDatabase')
  test.skip(reason !== null, reason ?? '')

  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = randomUUID()

  test.beforeAll(async ({ api }) => {
    // A remote that does not exist, so the reset's re-sync has nothing to
    // read — which is the WS-21 situation.
    const repository = await api.createRepository({
      name: `ws-override-${suffix}`,
      kind: 'template',
      remoteUrl: `https://github.com/aorwall/ws-override-${suffix}.git`,
    })
    ctx.repositoryId = repository.id

    const workspace = await api.createWorkspace({
      name: `Override target ${suffix}`,
      dockerImage: 'acme/from-git:1',
      repos: [{ repositoryId: repository.id, isPrimary: true }],
    })
    ctx.set('workspaceId', workspace.id)
  })

  test.afterAll(async ({ api }) => {
    const workspaceId = ctx.get<string>('workspaceId')
    if (workspaceId) {
      await api.deleteWorkspace(workspaceId).catch(() => {})
    }
    if (ctx.repositoryId) {
      await api.deleteRepository(ctx.repositoryId).catch(() => {})
    }
  })

  // ─── WS-20: A workspace never in git has nothing to reset to ─────────

  test('WS-20: A workspace never in git has nothing to reset to', async ({ api }) => {
    const response = await api.resetGitWorkspaceRaw(ctx.get<string>('workspaceId')!)

    expect(response.status).toBe(400)
  })

  // ─── WS-01: A git-declared workspace is read-only ────────────────────

  test('WS-01: A git-declared workspace cannot be edited', async ({ api }) => {
    const workspaceId = ctx.get<string>('workspaceId')!

    // Stand in for a sync of a repository that commits its workspaces.
    psql(
      `UPDATE workspaces SET source = 'git', source_repository_id = '${ctx.repositoryId}', ` +
        `source_key = 'api', source_config_path = '.moatless/workspaces.json' ` +
        `WHERE id = '${workspaceId}'`,
    )

    const updated = await api.updateWorkspaceRaw(workspaceId, {
      dockerImage: 'acme/edited:1',
    })
    expect(updated.status).toBe(409)
  })

  // ─── WS-02: Overriding with an empty patch takes ownership ───────────

  test('WS-02: Overriding with an empty patch takes ownership', async ({ api }) => {
    const workspace = await api.overrideWorkspace(ctx.get<string>('workspaceId')!)

    expect(workspace.source).toBe('manual')
    expect(workspace.syncedFromGit).toBe(false)
    // Taking ownership is not a change of configuration.
    expect(workspace.dockerImage).toBe('acme/from-git:1')

    ctx.set('overridden', workspace)
  })

  // ─── WS-03: The origin survives the override ─────────────────────────

  test('WS-03: The origin survives the override', async ({ api }) => {
    const workspace = await api.getWorkspace(ctx.get<string>('workspaceId')!)

    // Without these there would be nothing to reset back to.
    expect(workspace.sourceRepositoryId).toBe(ctx.repositoryId)
    expect(workspace.sourceKey).toBe('api')
    expect(workspace.sourceConfigPath).toBe('.moatless/workspaces.json')
  })

  // ─── WS-04: Editing is unlocked again ────────────────────────────────

  test('WS-04: Editing is unlocked again', async ({ api }) => {
    const workspace = await api.updateWorkspace(ctx.get<string>('workspaceId')!, {
      dockerImage: 'acme/edited:1',
    })

    expect(workspace.dockerImage).toBe('acme/edited:1')
  })

  // ─── WS-21: A reset that cannot reach git leaves the override alone ──

  test('WS-21: A reset that cannot reach git leaves the override alone', async ({ api }) => {
    const reset = await api.resetGitWorkspace(ctx.get<string>('workspaceId')!)

    expect(reset.resynced).toBe(false)
    expect(reset.error).toBeTruthy()
    // A workspace must never be left claiming a provenance it does not have.
    expect(reset.workspace.source).toBe('manual')
    expect(reset.workspace.dockerImage).toBe('acme/edited:1')
  })

  // ─── WS-08: Deleting a git-origin workspace leaves a tombstone ───────

  test('WS-08: Deleting a git-origin workspace leaves a tombstone', async ({ api }) => {
    const workspaceId = ctx.get<string>('workspaceId')!

    await api.deleteWorkspace(workspaceId)

    // The row survives the delete so a later sync cannot quietly recreate the
    // same source key — reclaiming it takes an explicit reset.
    const tombstone = await api.getWorkspace(workspaceId)
    expect(tombstone.deleted).toBe(true)
    expect(tombstone.source).toBe('manual')
    expect(tombstone.sourceRepositoryId).toBe(ctx.repositoryId)
    expect(tombstone.sourceKey).toBe('api')
  })
})
