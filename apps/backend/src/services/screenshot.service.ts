import { promises as fs } from 'node:fs'
import path from 'node:path'
import { type Browser, chromium } from '@playwright/test'
import sharp from 'sharp'
import { getDataDir } from '../utils/data-dir.js'
import { createTaskLogger } from '../utils/logger.js'

export class ScreenshotService {
  private browser: Browser | undefined
  private logger = createTaskLogger('screenshot-service')

  async ensureBrowser(): Promise<Browser> {
    if (!this.browser) {
      this.logger.debug('Launching Playwright browser')
      this.browser = await chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
      })
    }
    return this.browser
  }

  async captureScreenshot(
    url: string,
    taskId: string,
    iterationNumber: number,
  ): Promise<{ screenshotPath: string; thumbnailPath: string }> {
    const logger = this.logger.child({ taskId, iterationNumber, url })
    logger.info('Starting screenshot capture')

    const browser = await this.ensureBrowser()
    const page = await browser.newPage()

    try {
      // Set viewport to standard desktop size
      await page.setViewportSize({ width: 1280, height: 720 })
      logger.debug('Set viewport size to 1280x720')

      // Navigate to URL with timeout
      logger.debug('Navigating to URL')
      await page.goto(url, {
        waitUntil: 'networkidle',
        timeout: 30000,
      })
      logger.debug('Page loaded, waiting for stabilization')

      // Wait a bit for any animations/transitions to complete
      await page.waitForTimeout(2000)

      // Prepare screenshot path
      const screenshotDir = path.join(getDataDir(), 'screenshots', taskId)
      await fs.mkdir(screenshotDir, { recursive: true })
      logger.debug('Created screenshot directory', { screenshotDir })

      const screenshotPath = path.join(screenshotDir, `iteration-${iterationNumber}.png`)

      // Capture screenshot
      logger.debug('Capturing screenshot')
      await page.screenshot({
        path: screenshotPath,
        fullPage: false, // Just viewport
        type: 'png',
      })

      // Generate thumbnail with 340px width
      const thumbnailPath = path.join(screenshotDir, `iteration-${iterationNumber}-thumb.png`)
      logger.debug('Generating thumbnail')

      await sharp(screenshotPath)
        .resize(340, null, {
          withoutEnlargement: true,
          fit: 'inside',
        })
        .png()
        .toFile(thumbnailPath)

      logger.info('Screenshot and thumbnail captured successfully', {
        path: screenshotPath,
        thumbnailPath,
      })

      return { screenshotPath, thumbnailPath }
    } catch (error) {
      logger.error('Failed to capture screenshot', error as Error)
      throw error
    } finally {
      await page.close()
      logger.debug('Closed page')
    }
  }

  async cleanup(): Promise<void> {
    if (this.browser) {
      this.logger.debug('Closing Playwright browser')
      await this.browser.close()
      this.browser = undefined
    }
  }
}

// Create singleton instance
let screenshotService: ScreenshotService | undefined

export function getScreenshotService(): ScreenshotService {
  if (!screenshotService) {
    screenshotService = new ScreenshotService()
  }
  return screenshotService
}

export async function cleanupScreenshotService(): Promise<void> {
  if (screenshotService) {
    await screenshotService.cleanup()
    screenshotService = undefined
  }
}
