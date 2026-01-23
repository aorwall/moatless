/**
 * Browser automation types for sandbox
 *
 * Minimal types for browser lifecycle and screenshot serving.
 * All page management is handled client-side via CDP.
 */

/**
 * Options for starting the browser server
 */
export interface BrowserServerOptions {
  /** HTTP API port (default: 9222) */
  port?: number
  /** CDP port for Playwright connections (default: 9223) */
  cdpPort?: number
  /** Run browser in headless mode (default: true) */
  headless?: boolean
  /** Persistent user data directory */
  userDataDir?: string
}

/**
 * Browser server info returned from status endpoint
 */
export interface BrowserServerInfo {
  /** Whether the browser server is running */
  running: boolean
  /** WebSocket endpoint for CDP connections */
  wsEndpoint?: string
  /** HTTP API port */
  httpPort?: number
  /** CDP port */
  cdpPort?: number
}

/**
 * Browser action types for screenshot metadata
 */
export type BrowserAction = 'navigate' | 'manual'

/**
 * Screenshot metadata stored with screenshot files
 * Written by client scripts, read by server for listing
 */
export interface ScreenshotMetadata {
  /** Filename in screenshots directory */
  filename: string
  /** ISO timestamp when captured */
  timestamp: string
  /** Action that triggered the screenshot */
  action: BrowserAction
  /** Page name (client-side identifier) */
  pageName: string
  /** Page URL at capture time */
  url: string
  /** Screenshot width in pixels */
  width: number
  /** Screenshot height in pixels */
  height: number
  /** Action-specific context (selector clicked, text filled, etc.) */
  details?: Record<string, unknown>
}

/**
 * Response for listing screenshots
 */
export interface ScreenshotListResponse {
  screenshots: ScreenshotMetadata[]
  total: number
}
