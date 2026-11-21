/**
 * Integration step functions.
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type { MoatApi } from '../../shared/moat-api'
import type { TaskLoopFixtureResponse } from '../../shared/types'

// =============================================================================
// INT-01: Create subscription loop fixture
// =============================================================================

export async function createSubscriptionLoopFixture(
  api: MoatApi,
  workspaceId: string,
  options?: {
    sourceId?: string
    sourceName?: string
    routingMode?: string
    promptTemplate?: string
    adapterConfig?: Record<string, unknown>
  },
): Promise<{ triggerId: string; loopId: string; config: TaskLoopFixtureResponse }> {
  const config = await api.createTaskTriggerFixture({
    adapterKind: 'slack',
    sourceId: options?.sourceId ?? `subscription-${Date.now()}`,
    sourceName: options?.sourceName,
    workspaceId,
    routingMode: options?.routingMode ?? 'by_subject',
    promptTemplate: options?.promptTemplate,
    adapterConfig: options?.adapterConfig,
  })
  return { triggerId: config.id, loopId: config.id, config }
}
