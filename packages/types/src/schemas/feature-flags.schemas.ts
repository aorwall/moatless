import { z } from 'zod'

/**
 * Schema for Slack-related configuration
 */
export const SlackConfigSchema = z.object({
  teamId: z.string().optional().describe('Default Slack workspace/team ID'),
})

/**
 * Schema for feature flags response
 * Keys are lowercase snake_case versions of FeatureFlagKey
 */
export const FeatureFlagsResponseSchema = z.object({
  netlify_publish: z.boolean().describe('Whether Netlify publishing features are enabled'),
  figma_clipboard_paste: z.boolean().describe('Whether Figma clipboard paste detection is enabled'),
  figma_images_in_prompt: z
    .boolean()
    .describe('Whether Figma images are copied to workspace for agent context'),
  show_model_selector: z
    .boolean()
    .describe('Whether model selector is visible in task creation UIs'),
  slack_channel: z
    .boolean()
    .describe('Whether Slack channel integration is enabled for repository mapping'),
  slack: SlackConfigSchema.optional().describe('Slack integration configuration'),
})

/**
 * Type for the feature flags response
 */
export type FeatureFlagsResponse = z.infer<typeof FeatureFlagsResponseSchema>
