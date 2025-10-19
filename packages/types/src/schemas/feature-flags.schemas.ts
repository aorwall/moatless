import { z } from 'zod'

/**
 * Schema for feature flags response
 * Keys are lowercase snake_case versions of FeatureFlagKey
 */
export const FeatureFlagsResponseSchema = z.object({
  netlify_publish: z.boolean().describe('Whether Netlify publishing features are enabled'),
  show_system_settings: z
    .boolean()
    .describe('Whether system configuration sections (General, Claude, MCP) are visible'),
  figma_clipboard_paste: z.boolean().describe('Whether Figma clipboard paste detection is enabled'),
  figma_images_in_prompt: z
    .boolean()
    .describe('Whether Figma images are copied to workspace for agent context'),
  show_model_selector: z
    .boolean()
    .describe('Whether model selector is visible in task creation UIs'),
})

/**
 * Type for the feature flags response
 */
export type FeatureFlagsResponse = z.infer<typeof FeatureFlagsResponseSchema>
