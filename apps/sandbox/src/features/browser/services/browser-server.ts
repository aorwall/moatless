/**
 * Browser Server - Minimal Chromium lifecycle management
 *
 * Manages a persistent browser instance and provides:
 * - WebSocket endpoint for CDP connections (allowing full Playwright access)
 * - Screenshot directory serving (scripts write directly to disk)
 *
 * All page management and actions are handled client-side via CDP.
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import { SANDBOX_DATA_DIR } from '@moatless/sandbox-schemas'
import type { Browser } from 'playwright'
import { logger as rootLogger } from '../../../shared/logger'
import type {
  BrowserServerInfo,
  BrowserServerOptions,
  ScreenshotMetadata,
} from '../types/browser.types'

const logger = rootLogger.child({ service: 'browser-server' })

/**
 * BrowserServer manages a persistent Chromium browser for automation
 */
export class BrowserServer {
  private browser: Browser | undefined
  private wsEndpoint: string | undefined
  private readonly options: Required<BrowserServerOptions>
  private chromium: typeof import('playwright').chromium | undefined
  private available = false
  private unavailableReason: string | undefined
  private screenshotsDir: string

  constructor(options: BrowserServerOptions = {}) {
    this.options = {
      port: options.port ?? 9222,
      cdpPort: options.cdpPort ?? 9223,
      headless: options.headless ?? true,
      userDataDir: options.userDataDir ?? '',
    }

    // Initialize screenshots directory based on DATA_DIR
    const dataDir = process.env['DATA_DIR'] || SANDBOX_DATA_DIR
    this.screenshotsDir = path.join(dataDir, 'screenshots')
  }

  /**
   * Initialize Playwright (check availability)
   */
  private async initPlaywright(): Promise<void> {
    if (this.chromium) {
      return // Already initialized
    }

    try {
      const playwright = await import('playwright')
      this.chromium = playwright.chromium

      // Check if chromium executable exists
      const executablePath = this.chromium.executablePath()
      const { promises: fs } = await import('node:fs')
      const exists = await fs
        .access(executablePath)
        .then(() => true)
        .catch(() => false)

      if (exists) {
        this.available = true
        logger.info({ executablePath }, 'Playwright chromium available')
      } else {
        this.unavailableReason = `Chromium not found at ${executablePath}`
        logger.warn({ executablePath }, 'Playwright chromium not available')
      }
    } catch (error) {
      this.unavailableReason = `Playwright not installed: ${error instanceof Error ? error.message : String(error)}`
      logger.warn({ error }, 'Playwright not available')
    }
  }

  /**
   * Check if browser server is available
   */
  isAvailable(): boolean {
    return this.available
  }

  /**
   * Get reason why browser server is unavailable
   */
  getUnavailableReason(): string | undefined {
    return this.unavailableReason
  }

  /**
   * Wait for CDP endpoint to be ready
   */
  private async waitForCdpReady(): Promise<void> {
    const maxAttempts = 10
    const delayMs = 500

    for (let i = 0; i < maxAttempts; i++) {
      try {
        const response = await fetch(`http://localhost:${this.options.cdpPort}/json/version`)
        if (response.ok) {
          return // CDP is ready
        }
      } catch {
        // CDP endpoint not ready yet, retry
      }
      await new Promise((resolve) => setTimeout(resolve, delayMs))
    }

    logger.warn('CDP endpoint did not respond to /json/version, proceeding anyway')
  }

  /**
   * Start the browser server
   */
  async start(): Promise<{ wsEndpoint: string }> {
    if (this.browser) {
      logger.info('Browser server already running')
      return { wsEndpoint: this.wsEndpoint! }
    }

    await this.initPlaywright()

    if (!(this.available && this.chromium)) {
      throw new Error(this.unavailableReason || 'Playwright not available')
    }

    logger.info(
      {
        headless: this.options.headless,
        cdpPort: this.options.cdpPort,
      },
      'Starting browser server',
    )

    try {
      // Launch browser with CDP debugging port
      const launchArgs = [
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--disable-dev-shm-usage',
        `--remote-debugging-port=${this.options.cdpPort}`,
      ]

      this.browser = await this.chromium.launch({
        headless: this.options.headless,
        args: launchArgs,
      })

      // Wait for CDP to be ready
      await this.waitForCdpReady()

      // Return HTTP endpoint - connectOverCDP() will fetch wsEndpoint internally
      // This avoids issues with incomplete WebSocket URLs
      this.wsEndpoint = `http://localhost:${this.options.cdpPort}`

      logger.info(
        {
          wsEndpoint: this.wsEndpoint,
          cdpPort: this.options.cdpPort,
        },
        'Browser server started',
      )

      return { wsEndpoint: this.wsEndpoint }
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to start browser server')
      throw error
    }
  }

  /**
   * Stop the browser server and clean up all resources
   */
  async stop(): Promise<void> {
    if (!this.browser) {
      logger.info('Browser server not running')
      return
    }

    logger.info('Stopping browser server')

    try {
      await this.browser.close()
      this.browser = undefined
      this.wsEndpoint = undefined

      logger.info('Browser server stopped')
    } catch (error) {
      logger.error({ err: error as Error }, 'Error stopping browser server')
      throw error
    }
  }

  /**
   * Check if browser server is running
   */
  isRunning(): boolean {
    return this.browser?.isConnected() ?? false
  }

  /**
   * Get browser server info
   */
  getInfo(): BrowserServerInfo {
    return {
      running: this.isRunning(),
      wsEndpoint: this.wsEndpoint,
      httpPort: this.options.port,
      cdpPort: this.options.cdpPort,
    }
  }

  /**
   * Get the WebSocket endpoint for CDP connections
   */
  getWsEndpoint(): string | undefined {
    return this.wsEndpoint
  }

  // ==================== Screenshot Directory Serving ====================

  /**
   * Get screenshots directory path
   */
  getScreenshotsDir(): string {
    return this.screenshotsDir
  }

  /**
   * List screenshots by reading metadata JSON files from disk
   */
  async listScreenshots(): Promise<ScreenshotMetadata[]> {
    try {
      // Ensure directory exists
      await fs.mkdir(this.screenshotsDir, { recursive: true })

      const files = await fs.readdir(this.screenshotsDir)
      const screenshots: ScreenshotMetadata[] = []

      for (const file of files) {
        if (file.endsWith('.png.json')) {
          try {
            const metadataPath = path.join(this.screenshotsDir, file)
            const content = await fs.readFile(metadataPath, 'utf-8')
            const metadata = JSON.parse(content) as ScreenshotMetadata
            screenshots.push(metadata)
          } catch {
            // Skip invalid metadata files
          }
        }
      }

      // Sort by filename (which includes index prefix)
      screenshots.sort((a, b) => a.filename.localeCompare(b.filename))

      return screenshots
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to list screenshots')
      return []
    }
  }

  /**
   * Check if a screenshot file exists
   */
  async screenshotExists(filename: string): Promise<boolean> {
    try {
      await fs.access(path.join(this.screenshotsDir, filename))
      return true
    } catch {
      return false
    }
  }

  /**
   * Read a screenshot file
   */
  readScreenshot(filename: string): Promise<Buffer> {
    const filepath = path.join(this.screenshotsDir, filename)
    return fs.readFile(filepath)
  }

  /**
   * Clear all screenshots from disk
   */
  async clearScreenshots(): Promise<number> {
    try {
      await fs.mkdir(this.screenshotsDir, { recursive: true })

      const files = await fs.readdir(this.screenshotsDir)
      let count = 0

      for (const file of files) {
        if (file.endsWith('.png') || file.endsWith('.png.json')) {
          try {
            await fs.unlink(path.join(this.screenshotsDir, file))
            if (file.endsWith('.png')) {
              count++
            }
          } catch {
            // Ignore deletion errors
          }
        }
      }

      logger.info({ count }, 'Screenshots cleared')
      return count
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to clear screenshots')
      return 0
    }
  }
}

// Singleton instance for the sandbox
let browserServerInstance: BrowserServer | undefined

/**
 * Get or create the browser server instance
 */
export function getBrowserServer(options?: BrowserServerOptions): BrowserServer {
  if (!browserServerInstance) {
    browserServerInstance = new BrowserServer(options)
  }
  return browserServerInstance
}

/**
 * Start the browser server
 */
export async function startBrowserServer(
  options?: BrowserServerOptions,
): Promise<{ server: BrowserServer; wsEndpoint: string }> {
  const server = getBrowserServer(options)
  const { wsEndpoint } = await server.start()
  return { server, wsEndpoint }
}

/**
 * Stop the browser server
 */
export async function stopBrowserServer(): Promise<void> {
  if (browserServerInstance) {
    await browserServerInstance.stop()
    browserServerInstance = undefined
  }
}
