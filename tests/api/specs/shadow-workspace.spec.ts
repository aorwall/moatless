/**
 * Standalone: Shadow workspace primary (specs/workspaces/01-git-override-scenarios.md)
 *
 * A repository's shadow workspace `ws_<repo>` is written by two paths that used
 * to disagree. Creating a task inserts the placement under a deterministic id;
 * git sync deletes every placement and re-elects the primary under a random one.
 * Once sync had run, the deterministic id was gone, so the next task create no
 * longer deduped and added a second primary — a 500 on
 * `workspace_repos_one_primary`. The insert now stands down when a primary
 * already exists.
 *
 * Scenarios:
 *   WS-09: Creating a task after git sync re-elected the primary still works
 */

import { missingCapability } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { psql } from '../helpers/psql'
import { discardTask } from '../helpers/teardown'

test.describe('Standalone: Shadow workspace primary', () => {
  // The state under test is one only git sync produces, and reaching it any
  // other way means writing the placement rows directly.
  const reason = missingCapability('directDatabase')
  test.skip(reason !== null, reason ?? '')

  test('WS-09: Creating a task after git sync re-elected the primary still works', async ({
    api,
  }) => {
    // A repository of its own, so the shadow workspace starts clean.
    const unique = Date.now()
    const repository = await api.createRepository({
      name: `shadow-primary-regression-${unique}`,
      kind: 'template',
      remoteUrl: `https://github.com/aorwall/shadow-primary-regression-${unique}.git`,
    })
    const workspaceId = `ws_${repository.id}`
    const created: string[] = []

    try {
      const first = await api.createVibeTask({
        templateId: repository.id,
        name: 'first',
        message: 'shadow-primary regression',
      })
      created.push(first.taskId)

      // What git sync leaves behind: every placement dropped, the primary
      // re-inserted under a random id.
      psql(
        `DELETE FROM workspace_repos WHERE workspace_id = '${workspaceId}'; ` +
          'INSERT INTO workspace_repos (id, workspace_id, repository_id, is_primary, position) ' +
          `VALUES ('wsr_' || md5(random()::text), '${workspaceId}', '${repository.id}', true, 0)`,
      )

      // This is the create that used to 500.
      const second = await api.createVibeTask({
        templateId: repository.id,
        name: 'second',
        message: 'shadow-primary regression',
      })
      created.push(second.taskId)

      expect(
        psql(
          `SELECT count(*) FROM workspace_repos WHERE workspace_id = '${workspaceId}' AND is_primary`,
        ),
      ).toBe('1')
    } finally {
      for (const taskId of created) {
        await discardTask(api, taskId)
      }
      await api.deleteRepository(repository.id).catch(() => {})
    }
  })
})
