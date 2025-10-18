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
} as const

/**
 * Type representing all valid feature flag keys
 */
export type FeatureFlagKey = keyof typeof FEATURE_FLAGS

/**
 * Type representing the feature flag values as they appear in the API/runtime
 * Keys are lowercase with underscores (e.g., "netlify_publish")
 */
export type FeatureFlagValue = (typeof FEATURE_FLAGS)[FeatureFlagKey]

/**
 * Default values for feature flags when environment variables are not set
 */
export const FEATURE_FLAG_DEFAULTS: Record<FeatureFlagKey, boolean> = {
  NETLIFY_PUBLISH: false, // Disabled by default
}

/**
 * Type representing the runtime feature flags object
 * Keys are lowercase snake_case strings, values are booleans
 */
export type FeatureFlags = {
  [K in FeatureFlagKey as Lowercase<K>]: boolean
}
