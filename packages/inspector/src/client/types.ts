/// <reference lib="dom" />

/** Configuration for the inspector */
export interface InspectorConfig {
  HIGHLIGHT_COLOR: string
  HIGHLIGHT_BG: string
  DEBOUNCE_MS: number
  TOOLTIP_OFFSET: number
  Z: number
  MAX_TOOLTIP_WIDTH: number
  SCROLL_DEBOUNCE_MS: number
  ALLOWED_ORIGINS: string[]
  FULL_WIDTH_DELTA: number
  FEATURES: {
    NETWORK_MONITORING: boolean
    SCROLL_MONITORING: boolean
  }
}

/** Parsed component info from data-component-id attribute */
export interface ComponentInfo {
  componentId: string
  path: string
  line: number
  column: number
  componentName: string
}

/** Element bounding rect payload */
export interface ElementRect {
  top: number
  left: number
  width: number
  height: number
  bottom: number
  right: number
}

/** Network request payload */
export interface NetworkRequestPayload {
  url: string
  method: string
  status?: number
  statusText?: string
  error?: { message: string; stack?: string }
  duration: number
  timestamp: string
  origin: string
}

/** Scroll position payload */
export interface ScrollPositionPayload {
  scrollY: number
  scrollHeight: number
  clientHeight: number
  timestamp: number
}

/** Element clicked payload */
export interface ElementClickedPayload {
  componentId: string | null
  path: string | null
  line: number | null
  column: number | null
  componentName: string
  elementRect: ElementRect
  timestamp: number
}

/** Messages sent FROM inspector TO parent */
export type InspectorOutboundMessage =
  | { type: 'console'; level: string; message: string; args: unknown[]; stack?: string }
  | { type: 'NETWORK_REQUEST'; payload: NetworkRequestPayload }
  | { type: 'SCROLL_POSITION'; payload: ScrollPositionPayload }
  | { type: 'ROUTE_CHANGE'; payload: { path: string; timestamp: number } }
  | { type: 'ELEMENT_CLICKED'; payload: ElementClickedPayload }
  | {
      type: 'INSPECTOR_AVAILABILITY'
      payload: { isAvailable: boolean; elementCount: number; timestamp: number }
    }
  | { type: 'INSPECTOR_SCRIPT_LOADED'; payload: { version: string; serverId: string } }
  | { type: 'HEARTBEAT'; payload: { timestamp: number } }
  | { type: 'SELECTOR_STATE_RESPONSE'; payload: { isActive: boolean } }

/** Messages sent FROM parent TO inspector */
export type InspectorInboundMessageType =
  | 'TOGGLE_SELECTOR'
  | 'SELECTOR_STATE_REQUEST'
  | 'CHECK_AVAILABILITY_REQUEST'
  | 'CLEAR_SELECTION'
  | 'ADD_TO_SELECTION'
  | 'REMOVE_FROM_SELECTION'
  | 'NAVIGATE'

/** Element inspector public API */
export interface ElementInspectorAPI {
  toggle: (active: boolean) => void
  addToSelection: (componentId: string) => void
  removeFromSelection: (componentId: string) => void
  clearAllSelections: () => void
}

/** Internal logger that bypasses console interception */
export interface InternalLogger {
  log: (...args: unknown[]) => void
  warn: (...args: unknown[]) => void
  error: (...args: unknown[]) => void
  info: (...args: unknown[]) => void
  debug: (...args: unknown[]) => void
}
