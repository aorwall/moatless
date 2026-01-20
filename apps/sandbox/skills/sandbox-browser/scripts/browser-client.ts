/**
 * Standalone Browser Client for Sandbox Browser Automation
 *
 * This file is self-contained and can be imported directly from scripts
 * running inside the sandbox container without any @moatless/* dependencies.
 *
 * All page management happens locally via CDP - the server only handles:
 * - Browser lifecycle (start/stop)
 * - WebSocket endpoint for CDP connections
 * - Serving screenshot files from disk
 *
 * Usage:
 * ```typescript
 * import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";
 *
 * // Auto-discovers sandbox server from PORT env (default: 8001)
 * const client = await connect();
 * const page = await client.page("main");
 *
 * // Use getPreviewUrl() to navigate to the preview server
 * await page.goto(getPreviewUrl());           // http://localhost:3000
 * await page.goto(getPreviewUrl("/about"));   // http://localhost:3000/about
 * await waitForPageLoad(page);
 *
 * const snapshot = await client.getAISnapshot("main");
 * console.log(snapshot);
 *
 * await client.disconnect();
 * ```
 */

import { mkdir, writeFile } from 'node:fs/promises'
import type { Browser, BrowserContext, Page } from 'playwright'

// ============================================================================
// Types (embedded to avoid external dependencies)
// ============================================================================

/**
 * Browser action types that trigger screenshot capture
 */
export type BrowserAction = 'navigate' | 'click' | 'fill' | 'manual'

/**
 * Options for connecting to browser
 */
export interface ConnectOptions {
  /** Default viewport size (default: 1280x720) */
  viewport?: { width: number; height: number }
}

/**
 * Options for creating or getting a page
 */
export interface GetPageOptions {
  /** Default URL to navigate to if creating new page */
  url?: string
  /** Override viewport for this page */
  viewport?: { width: number; height: number }
  /** Disable automatic screenshot capture after actions (default: false) */
  disableAutoCapture?: boolean
}

/**
 * Options for waiting for page load
 */
export interface WaitForLoadOptions {
  /** Timeout in milliseconds (default: 30000) */
  timeout?: number
  /** Wait until condition (default: 'domcontentloaded') */
  waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'
}

/**
 * Screenshot metadata written to disk
 */
interface ScreenshotMetadata {
  filename: string
  action: BrowserAction
  pageName: string
  url: string
  timestamp: string
  width: number
  height: number
}

/**
 * Browser client interface for scripts
 */
export interface IBrowserClient {
  /** Get or create a named page - returns actual Playwright Page with auto-capture */
  page(name: string, options?: GetPageOptions): Promise<Page>

  /** List all named pages */
  list(): Promise<string[]>

  /** Close a named page */
  close(name: string): Promise<void>

  /** Disconnect from browser server */
  disconnect(): Promise<void>

  /** Get ARIA snapshot of a page */
  getAISnapshot(name: string): Promise<string>

  /** Manually capture a screenshot for a page */
  captureScreenshot(pageName: string, action?: BrowserAction): Promise<void>
}

// ============================================================================
// Browser Client Implementation
// ============================================================================

/**
 * Get the default sandbox server URL from environment.
 * Scripts running inside sandbox can use this to auto-discover the server.
 */
function getDefaultServerUrl(): string {
  const port = process.env['PORT'] || '8001'
  return `http://localhost:${port}`
}

/**
 * Get the preview server URL.
 * The preview server always runs on port 3000 inside the sandbox.
 * @param path - Optional path to append (e.g., "/about")
 * @returns The full URL to the preview server
 */
export function getPreviewUrl(path?: string): string {
  const baseUrl = 'http://localhost:3000'
  if (!path) {
    return baseUrl
  }
  // Ensure path starts with /
  const normalizedPath = path.startsWith('/') ? path : `/${path}`
  return `${baseUrl}${normalizedPath}`
}

/**
 * Browser client for automation scripts
 * Manages pages locally via CDP - server only provides browser lifecycle
 */
export class BrowserClient implements IBrowserClient {
  private browser: Browser | undefined
  private context: BrowserContext | undefined
  private pages: Map<string, Page> = new Map()
  private pageAutoCapture: Map<string, boolean> = new Map()
  private screenshotIndex = 0
  private readonly serverUrl: string
  private readonly screenshotDir: string
  private defaultViewport = { width: 1280, height: 720 }

  constructor(serverUrl?: string) {
    // Normalize URL (remove trailing slash), auto-discover if not provided
    const url = serverUrl || getDefaultServerUrl()
    this.serverUrl = url.replace(/\/$/, '')
    this.screenshotDir = process.env['DATA_DIR']
      ? `${process.env['DATA_DIR']}/screenshots`
      : '/data/screenshots'
  }

  /**
   * Capture a screenshot by writing directly to disk
   * @param pageName - Name of the page
   * @param action - Action that triggered the screenshot (default: 'manual')
   */
  async captureScreenshot(pageName: string, action: BrowserAction = 'manual'): Promise<void> {
    const page = this.pages.get(pageName)
    if (!page || page.isClosed()) {
      return
    }

    try {
      // Ensure directory exists
      await mkdir(this.screenshotDir, { recursive: true })

      // Generate filename with index, action, and timestamp
      const index = String(this.screenshotIndex++).padStart(3, '0')
      const timestamp = Date.now()
      const filename = `${index}_${action}_${timestamp}.png`
      const filepath = `${this.screenshotDir}/${filename}`

      // Take screenshot
      await page.screenshot({ path: filepath, type: 'png' })

      // Get viewport size
      const viewport = page.viewportSize() || this.defaultViewport

      // Write metadata JSON
      const metadata: ScreenshotMetadata = {
        filename,
        action,
        pageName,
        url: page.url(),
        timestamp: new Date().toISOString(),
        width: viewport.width,
        height: viewport.height,
      }
      await writeFile(`${filepath}.json`, JSON.stringify(metadata, null, 2))
    } catch {
      // Silently ignore capture errors - they shouldn't break automation
    }
  }

  /**
   * Wrap a Page object to auto-capture screenshots after key actions
   */
  private wrapPageWithAutoCapture(page: Page, pageName: string): Page {
    const captureAfter = async (action: BrowserAction) => {
      // Check if auto-capture is enabled for this page
      if (this.pageAutoCapture.get(pageName) === false) {
        return
      }
      await this.captureScreenshot(pageName, action)
    }

    // Wrap goto
    const originalGoto = page.goto.bind(page)
    page.goto = async (...args: Parameters<Page['goto']>) => {
      const result = await originalGoto(...args)
      await captureAfter('navigate')
      return result
    }

    // Wrap click
    const originalClick = page.click.bind(page)
    page.click = async (...args: Parameters<Page['click']>) => {
      await originalClick(...args)
      await captureAfter('click')
    }

    // Wrap fill
    const originalFill = page.fill.bind(page)
    page.fill = async (...args: Parameters<Page['fill']>) => {
      await originalFill(...args)
      await captureAfter('fill')
    }

    return page
  }

  /**
   * Connect to the browser server via CDP
   */
  async connect(options?: ConnectOptions): Promise<void> {
    if (options?.viewport) {
      this.defaultViewport = options.viewport
    }

    // Get server info to check if browser is running
    const infoResponse = await fetch(`${this.serverUrl}/browser`)
    if (!infoResponse.ok) {
      throw new Error(`Failed to get browser server info: ${infoResponse.statusText}`)
    }

    let info = await infoResponse.json()

    if (!info.running) {
      // Start the browser server
      const startResponse = await fetch(`${this.serverUrl}/browser/start`, {
        method: 'POST',
      })
      if (!startResponse.ok) {
        const error = await startResponse.json().catch(() => ({ error: 'Unknown error' }))
        throw new Error(
          `Failed to start browser server: ${error.error || startResponse.statusText}`,
        )
      }
      info = await startResponse.json()
    }

    if (!info.wsEndpoint) {
      throw new Error('Browser server is running but no WebSocket endpoint available')
    }

    // Connect to browser via CDP
    const { chromium } = await import('playwright')
    this.browser = await chromium.connectOverCDP(info.wsEndpoint)

    // Get or create browser context with default viewport
    const contexts = this.browser.contexts()
    if (contexts.length > 0) {
      this.context = contexts[0]
    } else {
      this.context = await this.browser.newContext({
        viewport: this.defaultViewport,
      })
    }
  }

  /**
   * Get or create a named page
   * Returns the actual Playwright Page object with auto-capture enabled by default
   */
  async page(name: string, options?: GetPageOptions): Promise<Page> {
    if (!(this.browser && this.context)) {
      throw new Error('Not connected to browser server. Call connect() first.')
    }

    // Check if we already have this page locally
    const existingPage = this.pages.get(name)
    if (existingPage && !existingPage.isClosed()) {
      return existingPage
    }

    // Create new page locally
    let page = await this.context.newPage()

    // Set custom viewport if specified
    if (options?.viewport) {
      await page.setViewportSize(options.viewport)
    }

    // Navigate to URL if provided
    if (options?.url) {
      await page.goto(options.url, {
        waitUntil: 'domcontentloaded',
        timeout: 30000,
      })
    }

    // Track auto-capture setting for this page
    this.pageAutoCapture.set(name, !options?.disableAutoCapture)

    // Wrap page with auto-capture (unless disabled)
    if (!options?.disableAutoCapture) {
      page = this.wrapPageWithAutoCapture(page, name)
    }

    // Cache the page
    this.pages.set(name, page)

    // Remove from cache when closed
    page.on('close', () => {
      this.pages.delete(name)
      this.pageAutoCapture.delete(name)
    })

    return page
  }

  /**
   * List all named pages
   */
  list(): Promise<string[]> {
    // Return locally tracked pages (filter out closed ones)
    const activePages: string[] = []
    for (const [name, page] of this.pages) {
      if (!page.isClosed()) {
        activePages.push(name)
      } else {
        this.pages.delete(name)
        this.pageAutoCapture.delete(name)
      }
    }
    return Promise.resolve(activePages)
  }

  /**
   * Close a named page
   */
  async close(name: string): Promise<void> {
    const page = this.pages.get(name)
    if (page && !page.isClosed()) {
      await page.close()
    }
    this.pages.delete(name)
    this.pageAutoCapture.delete(name)
  }

  /**
   * Disconnect from the browser server
   * Note: We disconnect but don't close the browser - it stays alive for next script
   */
  disconnect(): Promise<void> {
    // Clear local page caches
    this.pages.clear()
    this.pageAutoCapture.clear()

    // Disconnect from browser (don't close it - browser stays alive)
    if (this.browser) {
      this.browser.disconnect()
      this.browser = undefined
      this.context = undefined
    }
    return Promise.resolve()
  }

  /**
   * Get AI-friendly ARIA snapshot of a page
   * Done client-side using Playwright's built-in ariaSnapshot
   */
  async getAISnapshot(name: string): Promise<string> {
    const page = this.pages.get(name)
    if (!page || page.isClosed()) {
      throw new Error(`Page '${name}' not found. Call page() first.`)
    }

    // Use Playwright's built-in ariaSnapshot on the root locator
    const snapshot = await page.locator(':root').ariaSnapshot()

    if (!snapshot) {
      throw new Error('Failed to get accessibility snapshot - page may not be loaded')
    }

    return snapshot
  }
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Connect to a browser server
 * @param options - Connection options including viewport size
 */
export async function connect(options?: ConnectOptions): Promise<BrowserClient> {
  const client = new BrowserClient()
  await client.connect(options)
  return client
}

/**
 * Wait for page to be fully loaded
 * Combines multiple wait strategies for reliable page readiness
 */
export async function waitForPageLoad(page: Page, options: WaitForLoadOptions = {}): Promise<void> {
  const { timeout = 30000, waitUntil = 'domcontentloaded' } = options

  // Wait for the specified load state
  await page.waitForLoadState(waitUntil, { timeout })

  // Additional stabilization - wait for network to be idle briefly
  try {
    await page.waitForLoadState('networkidle', { timeout: 5000 })
  } catch {
    // Network idle timeout is acceptable - page might have long-polling
  }

  // Small delay for any final rendering
  await page.waitForTimeout(100)
}

/**
 * Take a screenshot and return as base64
 */
export async function takeScreenshot(
  page: Page,
  options?: { fullPage?: boolean },
): Promise<string> {
  const buffer = await page.screenshot({
    fullPage: options?.fullPage ?? false,
    type: 'png',
  })
  return buffer.toString('base64')
}
