/**
 * Result types for Playwright operations
 * These are returned on successful operations; failures throw PlaywrightError
 * Each result includes a formatted message ready for display
 */

/**
 * Result of navigating to a URL
 */
export interface NavigationResult {
  url: string
  title: string
  snapshot: string
  message: string
}

/**
 * Result of clicking an element
 */
export interface ClickResult {
  url: string
  title: string
  snapshot: string
  message: string
}

/**
 * Result of evaluating JavaScript in page context
 */
export interface EvaluationResult {
  url: string
  title: string
  result: unknown
  snapshot: string
  message: string
}

/**
 * Result of filling form fields
 */
export interface FormFillResult {
  url: string
  title: string
  snapshot: string
  message: string
}

/**
 * Result of taking an accessibility snapshot
 * Use this to understand page state/structure without visual representation
 */
export interface SnapshotResult {
  url: string
  title: string
  snapshot: string
  message: string
}

/**
 * Result of taking a screenshot
 * Use this when you need to see the actual visual appearance of the UI
 */
export interface ScreenshotResult {
  url: string
  title: string
  screenshotPath?: string
  screenshotBase64: string
}

/**
 * Console message captured from the page
 */
export interface ConsoleMessage {
  type: 'log' | 'info' | 'warn' | 'error' | 'debug'
  text: string
  timestamp: Date
}

/**
 * Network request captured from the page
 */
export interface NetworkRequest {
  url: string
  method: string
  status?: number
  timing?: number
  timestamp: Date
}

/**
 * Result of getting console logs
 */
export interface ConsoleLogsResult {
  messages: ConsoleMessage[]
  count: number
  message: string
}

/**
 * Result of getting network requests
 */
export interface NetworkRequestsResult {
  requests: NetworkRequest[]
  count: number
  message: string
}

/**
 * Result of waiting for an element
 */
export interface WaitResult {
  success: boolean
  snapshot: string
  message: string
}
