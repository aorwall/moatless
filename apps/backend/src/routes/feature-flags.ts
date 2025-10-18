import type { OpenAPIHono } from '@hono/zod-openapi'
import { getFeatureFlagsRoute } from '@moatless/api-contract'
import type { FeatureFlagsResponse } from '@moatless/types'
import type { AppBindings } from '../app.js'
import { getAllFeatureFlags } from '../utils/feature-flags.js'

export function registerFeatureFlagsRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * GET /api/v1/feature-flags
   * Get all feature flags with their current values
   */
  app.openapi(getFeatureFlagsRoute, (c) => {
    const flags = getAllFeatureFlags() as FeatureFlagsResponse
    return c.json(flags)
  })
}
