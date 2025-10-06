/**
 * Feature flags for development and debugging
 */
export const featureFlags = {
  /**
   * Enable debug logging for event stream
   */
  debugEventStream: {
    enabled: import.meta.env.VITE_DEBUG_EVENT_STREAM === 'true',
    description: 'Enable verbose logging for event stream debugging',
  },
}

export function isFeatureEnabled(flag: keyof typeof featureFlags): boolean {
  return featureFlags[flag]?.enabled
}
