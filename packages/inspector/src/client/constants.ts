import type { InspectorConfig } from './types'

export const CONFIG: InspectorConfig = {
  HIGHLIGHT_COLOR: '#0da2e7',
  HIGHLIGHT_BG: '#0da2e71a',
  DEBOUNCE_MS: 10,
  TOOLTIP_OFFSET: 25,
  Z: 2147483647,
  MAX_TOOLTIP_WIDTH: 200,
  SCROLL_DEBOUNCE_MS: 420,
  ALLOWED_ORIGINS: ['*'],
  FULL_WIDTH_DELTA: 12,
  FEATURES: {
    NETWORK_MONITORING: false,
    SCROLL_MONITORING: false,
  },
}
