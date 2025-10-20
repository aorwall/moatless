/* Configuration for preview inspector script */

import type { Config } from './types.js'

export const CONFIG: Config = {
  HIGHLIGHT_COLOR: '#0da2e7',
  HIGHLIGHT_BG: '#0da2e71a',
  DEBOUNCE_MS: 10,
  TOOLTIP_OFFSET: 25,
  Z: 2147483647,
  MAX_TOOLTIP_WIDTH: 200,
  SCROLL_DEBOUNCE_MS: 420,
  ALLOWED_ORIGINS: ['*'], // Allow all origins for iframe communication
  FULL_WIDTH_DELTA: 12,
  FEATURES: {
    NETWORK_MONITORING: false, // Disable network request monitoring
    SCROLL_MONITORING: false, // Disable scroll position monitoring
  },
}
