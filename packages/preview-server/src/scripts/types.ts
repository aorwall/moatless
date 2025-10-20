/* Type definitions for preview inspector script */

export interface Config {
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

export interface ConsoleMessage {
  type: 'console'
  level: 'log' | 'warn' | 'error' | 'info' | 'debug'
  message: string
  args: unknown[]
  stack?: string
}

export interface NetworkRequestPayload {
  url: string
  method: string
  status?: number
  statusText?: string
  duration: number
  timestamp: string
  origin: string
  error?: { message: string; stack?: string }
}

export interface ScrollPositionPayload {
  scrollY: number
  scrollHeight: number
  clientHeight: number
  timestamp: number
}

export interface RouteChangePayload {
  path: string
  timestamp: number
}

export interface InspectorAvailabilityPayload {
  isAvailable: boolean
  elementCount: number
  timestamp: number
}

export interface ElementRect {
  top: number
  left: number
  width: number
  height: number
  bottom: number
  right: number
}

export interface SourceInfo {
  componentId: string
  path: string
  line: number
  column: number
  componentName: string
}

export interface ElementClickedPayload {
  componentId: string | null
  path: string | null
  line: number | null
  column: number | null
  componentName: string
  elementRect: ElementRect
  timestamp: number
}

export interface InspectorScriptLoadedPayload {
  version: string
  serverId: string
}

export interface HeartbeatPayload {
  timestamp: number
}

export interface SelectorStateResponsePayload {
  isActive: boolean
}

export type MessagePayload =
  | ConsoleMessage
  | { type: 'NETWORK_REQUEST'; payload: NetworkRequestPayload }
  | { type: 'SCROLL_POSITION'; payload: ScrollPositionPayload }
  | { type: 'ROUTE_CHANGE'; payload: RouteChangePayload }
  | { type: 'INSPECTOR_AVAILABILITY'; payload: InspectorAvailabilityPayload }
  | { type: 'ELEMENT_CLICKED'; payload: ElementClickedPayload }
  | { type: 'INSPECTOR_SCRIPT_LOADED'; payload: InspectorScriptLoadedPayload }
  | { type: 'HEARTBEAT'; payload: HeartbeatPayload }
  | { type: 'SELECTOR_STATE_RESPONSE'; payload: SelectorStateResponsePayload }

export interface MessageWithSource extends MessagePayload {
  source: 'preview-iframe'
}

export interface NavigatePayload {
  path: string
}

export interface IncomingMessage {
  type:
    | 'TOGGLE_SELECTOR'
    | 'SELECTOR_STATE_REQUEST'
    | 'CHECK_AVAILABILITY_REQUEST'
    | 'CLEAR_SELECTION'
    | 'NAVIGATE'
  payload?: boolean | NavigatePayload | string
}

export interface OverlayController {
  toggle: (state: boolean) => void
  clearSelection: () => void
}

export interface Logger {
  log: (...args: unknown[]) => void
  warn: (...args: unknown[]) => void
  error: (...args: unknown[]) => void
  info: (...args: unknown[]) => void
  debug: (...args: unknown[]) => void
}
