import { FEATURE_FLAG_DEFAULTS, FEATURE_FLAGS, type FeatureFlagKey } from '@moatless/types'

/**
 * Check if a feature flag is enabled
 *
 * Reads from environment variable FF_<FLAG_NAME>
 * Falls back to default value if not set
 *
 * @param flag - The feature flag key to check
 * @returns true if the feature is enabled, false otherwise
 *
 * @example
 * ```typescript
 * if (isFeatureEnabled('NETLIFY_PUBLISH')) {
 *   // Show publish features
 * }
 * ```
 */
export function isFeatureEnabled(flag: FeatureFlagKey): boolean {
  const envKey = `FF_${flag}`
  const envValue = process.env[envKey]

  if (envValue !== undefined) {
    return envValue === 'true' || envValue === '1'
  }

  return FEATURE_FLAG_DEFAULTS[flag]
}

/**
 * Get all feature flags with their current values
 *
 * Returns an object with lowercase snake_case keys matching the API format
 *
 * @returns Object mapping flag names to boolean values
 *
 * @example
 * ```typescript
 * const flags = getAllFeatureFlags()
 * // { netlify_publish: false, ... }
 * ```
 */
export function getAllFeatureFlags(): Record<string, boolean> {
  const flags: Record<string, boolean> = {}

  for (const [key] of Object.entries(FEATURE_FLAGS)) {
    const flagKey = key as FeatureFlagKey
    // Convert to lowercase for API consistency
    flags[flagKey.toLowerCase()] = isFeatureEnabled(flagKey)
  }

  return flags
}
