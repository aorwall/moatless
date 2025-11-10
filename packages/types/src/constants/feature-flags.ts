/**
 * Feature flags for controlling application features
 *
 * Feature flags can be controlled via environment variables:
 * - Format: FF_<FLAG_NAME>=true|false
 * - Example: FF_NETLIFY_PUBLISH=true
 *
 * Flags are evaluated on server startup and exposed to frontend via API.
 */

/**
 * Feature flag keys
 * These are the canonical names used in code
 */
export const FEATURE_FLAGS = {
  /**
   * Controls whether Netlify publishing features are available
   * - Settings: Shows/hides the Publish configuration section
   * - Preview: Shows/hides the Publish button
   */
  NETLIFY_PUBLISH: 'NETLIFY_PUBLISH',
  /**
   * Controls whether system configuration sections are visible
   * - Settings: Shows/hides General, Claude, and MCP Servers sections
   * - Useful for managed deployments where system settings are controlled by admins
   */
  SHOW_SYSTEM_SETTINGS: 'SHOW_SYSTEM_SETTINGS',
  /**
   * Controls whether Figma clipboard paste detection is enabled
   * - When enabled, detects and processes Figma clipboard data on paste
   * - When disabled, all clipboard paste events are handled normally (no Figma interception)
   * - Disabled by default for safety and to avoid unintended paste behavior
   */
  FIGMA_CLIPBOARD_PASTE: 'FIGMA_CLIPBOARD_PASTE',
  /**
   * Controls whether model selector is visible in task creation UIs
   * - When enabled, users can select between available Claude models
   * - When disabled, hides the model selector and uses Sonnet 4.5 as the default model
   * - Enabled by default to allow model selection
   */
  SHOW_MODEL_SELECTOR: 'SHOW_MODEL_SELECTOR',
} as const

/**
 * Type representing all valid feature flag keys
 */
export type FeatureFlagKey = keyof typeof FEATURE_FLAGS

/**
 * Default values for feature flags when environment variables are not set
 */
export const FEATURE_FLAG_DEFAULTS: Record<FeatureFlagKey, boolean> = {
  NETLIFY_PUBLISH: false, // Disabled by default
  SHOW_SYSTEM_SETTINGS: true, // Enabled by default - system settings visible
  FIGMA_CLIPBOARD_PASTE: false, // Disabled by default for safety
  SHOW_MODEL_SELECTOR: true, // Enabled by default - model selector visible
}
