/**
 * Browser automation feature for sandbox
 *
 * Provides Playwright-based browser automation with:
 * - Persistent browser server lifecycle management
 * - HTTP API for browser start/stop and screenshot serving
 * - CDP connections for full Playwright access from scripts
 *
 * Note: Page management and ARIA snapshots are handled client-side via CDP.
 * See apps/sandbox/skills/sandbox-browser/ for the client library.
 */

// API routes
export { registerBrowserRoutes } from './api/browser.routes'

// Services
export {
  BrowserServer,
  getBrowserServer,
  startBrowserServer,
  stopBrowserServer,
} from './services/browser-server'

// Types
export type {
  BrowserAction,
  BrowserServerInfo,
  BrowserServerOptions,
  ScreenshotListResponse,
  ScreenshotMetadata,
} from './types/browser.types'
