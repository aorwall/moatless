/**
 * Standalone: Integration Error Cases
 *
 * Independent error scenario tests for integration operations.
 *
 * INT-53: Create config with non-existent workspace
 */

import { expect, test } from '../fixtures/api-fixtures'

// ─── INT-53: Create config with non-existent workspace ───────────────

test('INT-53: Create config with non-existent workspace returns error', async ({ api }) => {
  const response = await api.createTaskTriggerFixtureRaw({
    adapterKind: 'slack',
    sourceId: `no-workspace-${Date.now()}`,
    workspaceId: 'non-existent-workspace-id',
    routingMode: 'by_subject',
  })

  // The rule names a workspace that is not there, so the trigger is refused
  // rather than created and left pointing at nothing.
  expect(response.status).toBe(400)
})
