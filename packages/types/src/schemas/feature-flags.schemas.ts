import { z } from 'zod'

/**
 * Schema for feature flags response
 * Keys are lowercase snake_case versions of FeatureFlagKey
 */
export const FeatureFlagsResponseSchema = z.object({
  netlify_publish: z.boolean().describe('Whether Netlify publishing features are enabled'),
})

/**
 * Type for the feature flags response
 */
export type FeatureFlagsResponse = z.infer<typeof FeatureFlagsResponseSchema>
