import { promises as fs } from 'node:fs'
import type { Browser, Page } from 'playwright'
import { logger } from '../../../shared/logger'

/**
 * Simple Playwright screenshot service for sandbox
 * Takes screenshots of preview servers without complex state management
 */
export class PlaywrightScreenshotService {
  private readonly logger = logger.child({ service: 'playwright-screenshot' })
  private chromium: typeof import('playwright').chromium | undefined
  private available = false
  private unavailableReason: string | undefined

  /**
   * Initialize the service by checking Playwright availability
   */
  async initialize(): Promise<void> {
    try {
      // Dynamic import - fails gracefully if Playwright not installed
      const playwright = await import('playwright')
      this.chromium = playwright.chromium

      // Check if chromium is installed
      const executablePath = this.chromium.executablePath()
      const exists = await fs
        .access(executablePath)
        .then(() => true)
        .catch(() => false)

      if (exists) {
        this.available = true
        this.logger.info({ executablePath }, 'Playwright chromium available')
      } else {
        this.unavailableReason = `Chromium not found at ${executablePath}`
        this.logger.warn({ executablePath }, 'Playwright chromium not available')
      }
    } catch (error) {
      this.unavailableReason = `Playwright not installed: ${error instanceof Error ? error.message : String(error)}`
      this.logger.warn({ error }, 'Playwright not available')
    }
  }

  /**
   * Check if Playwright is available
   */
  isAvailable(): boolean {
    return this.available
  }

  /**
   * Get reason why Playwright is unavailable
   */
  getUnavailableReason(): string | undefined {
    return this.unavailableReason
  }

  /**
   * Take screenshot of a URL
   * @param url - Full URL to screenshot (e.g., http://localhost:5173/)
   * @returns Base64-encoded PNG screenshot
   */
  async takeScreenshot(
    url: string,
  ): Promise<{ screenshotBase64: string; url: string; title: string }> {
    if (!(this.available && this.chromium)) {
      throw new Error(this.unavailableReason || 'Playwright not available')
    }

    let browser: Browser | undefined
    let page: Page | undefined

    try {
      this.logger.info({ url }, 'Taking screenshot')

      // Launch browser
      browser = await this.chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
      })

      // Create page
      page = await browser.newPage()
      await page.setViewportSize({ width: 1024, height: 720 })

      // Navigate to URL
      await page.goto(url, {
        waitUntil: 'domcontentloaded',
        timeout: 30000,
      })

      // Wait for page to stabilize
      await page.waitForTimeout(2000)

      // Take screenshot
      const screenshot = await page.screenshot({
        type: 'png',
        fullPage: false,
      })

      // Convert to base64
      const screenshotBase64 = screenshot.toString('base64')

      // Get page info
      const pageTitle = await page.title()

      this.logger.info(
        {
          url,
          title: pageTitle,
          screenshotSize: screenshot.length,
        },
        'Screenshot captured successfully',
      )

      return {
        screenshotBase64,
        url,
        title: pageTitle,
      }
    } catch (error) {
      this.logger.error({ err: error as Error, url }, 'Failed to take screenshot')
      throw new Error(
        `Failed to take screenshot: ${error instanceof Error ? error.message : String(error)}`,
      )
    } finally {
      // Clean up
      if (page && !page.isClosed()) {
        await page.close().catch(() => {
          // Ignore errors during cleanup
        })
      }
      if (browser) {
        await browser.close().catch(() => {
          // Ignore errors during cleanup
        })
      }
    }
  }
}

/**
 * Factory function to create PlaywrightScreenshotService instance
 */
export async function createPlaywrightScreenshotService(): Promise<PlaywrightScreenshotService> {
  const service = new PlaywrightScreenshotService()
  await service.initialize()
  return service
}
