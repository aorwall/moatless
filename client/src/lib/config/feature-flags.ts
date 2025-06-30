/**
 * Feature flags for controlling the global event stream migration
 */
export const featureFlags = {
  /**
   * Use global event stream instead of task-specific streams
   * When enabled:
   * - Task detail pages will use global stream
   * - Sidebar and task lists already use global stream
   * - Task-specific SSE connections will be disabled
   */
  useGlobalEventStream: {
    enabled: import.meta.env.VITE_USE_GLOBAL_EVENT_STREAM === 'true' || false,
    description: 'Use global event stream for all real-time updates',
  },

  /**
   * Enable debug logging for event stream
   */
  debugEventStream: {
    enabled: import.meta.env.VITE_DEBUG_EVENT_STREAM === 'true' || false,
    description: 'Enable verbose logging for event stream debugging',
  },
}

export function isFeatureEnabled(flag: keyof typeof featureFlags): boolean {
  return featureFlags[flag]?.enabled || false
}
