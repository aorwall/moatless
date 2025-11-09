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
 * Result of taking a screenshot/snapshot
 * Note: screenshotBase64 is required when includeScreenshot=true
 */
export interface SnapshotResult {
  url: string
  title: string
  screenshotPath?: string
  screenshotBase64: string
}
