import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { TaskIteration } from '@moatless/types'
import { type Browser, chromium } from '@playwright/test'
import sharp from 'sharp'
import type { StorageBackend } from '../storage/index.js'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'
import type { EventService } from './event.service.js'
import type { RepositoryService } from './repository.service.js'
import type { WorkspaceService } from './workspace.service.js'

export class ScreenshotService {
  private browser: Browser | undefined
  private serviceLogger = logger.child({ service: 'screenshot' })

  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private repositoryService: RepositoryService,
    private workspaceService: WorkspaceService,
  ) {}

  async ensureBrowser(): Promise<Browser> {
    if (!this.browser) {
      this.serviceLogger.debug('Launching Playwright browser')
      this.browser = await chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
      })
    }
    return this.browser
  }

  async captureSimpleScreenshot(url: string, routePath?: string): Promise<string> {
    // Construct full URL with optional path
    const fullUrl = routePath ? new URL(routePath, url).toString() : url

    const screenshotLogger = this.serviceLogger.child({ url: fullUrl, routePath })
    screenshotLogger.info('Starting simple screenshot capture')

    const browser = await this.ensureBrowser()
    const page = await browser.newPage()

    try {
      // Set viewport to standard desktop size
      await page.setViewportSize({ width: 1024, height: 720 })
      screenshotLogger.debug('Set viewport size to 1024x720')

      // Navigate to URL with timeout
      screenshotLogger.debug('Navigating to URL')
      await page.goto(fullUrl, {
        waitUntil: 'networkidle',
        timeout: 30000,
      })
      screenshotLogger.debug('Page loaded, waiting for stabilization')

      // Wait a bit for any animations/transitions to complete
      await page.waitForTimeout(2000)

      // Prepare screenshot path with UUID
      const screenshotDir = path.join(getDataDir(), 'screenshots')
      await fs.mkdir(screenshotDir, { recursive: true })
      screenshotLogger.debug('Created screenshot directory', { screenshotDir })

      const filename = `${randomUUID()}.png`
      const screenshotPath = path.join(screenshotDir, filename)

      // Capture screenshot
      screenshotLogger.debug('Capturing screenshot', { filename })
      await page.screenshot({
        path: screenshotPath,
        fullPage: false, // Just viewport
        type: 'png',
      })

      screenshotLogger.info('Screenshot captured successfully', {
        path: screenshotPath,
      })

      return screenshotPath
    } catch (error) {
      screenshotLogger.error('Failed to capture screenshot', error as Error)
      throw error
    } finally {
      await page.close()
      screenshotLogger.debug('Closed page')
    }
  }

  async captureScreenshot(
    url: string,
    taskId: string,
    iterationNumber: number,
  ): Promise<{ screenshotPath: string; thumbnailPath: string }> {
    const taskLogger = this.serviceLogger.child({ taskId, iterationNumber, url })
    taskLogger.info('Starting screenshot capture')

    const browser = await this.ensureBrowser()
    const page = await browser.newPage()

    try {
      // Set viewport to standard desktop size
      await page.setViewportSize({ width: 1024, height: 720 })
      taskLogger.debug('Set viewport size to 1280x720')

      // Navigate to URL with timeout
      taskLogger.debug('Navigating to URL')
      await page.goto(url, {
        waitUntil: 'networkidle',
        timeout: 30000,
      })
      taskLogger.debug('Page loaded, waiting for stabilization')

      // Wait a bit for any animations/transitions to complete
      await page.waitForTimeout(2000)

      // Prepare screenshot path
      const screenshotDir = path.join(getDataDir(), 'screenshots', taskId)
      await fs.mkdir(screenshotDir, { recursive: true })
      taskLogger.debug('Created screenshot directory', { screenshotDir })

      const screenshotPath = path.join(screenshotDir, `iteration-${iterationNumber}.png`)

      // Capture screenshot
      taskLogger.debug('Capturing screenshot')
      await page.screenshot({
        path: screenshotPath,
        fullPage: false, // Just viewport
        type: 'png',
      })

      // Generate thumbnail with 340px width
      const thumbnailPath = path.join(screenshotDir, `iteration-${iterationNumber}-thumb.png`)
      taskLogger.debug('Generating thumbnail')

      await sharp(screenshotPath)
        .resize(340, null, {
          withoutEnlargement: true,
          fit: 'inside',
        })
        .png()
        .toFile(thumbnailPath)

      taskLogger.info('Screenshot and thumbnail captured successfully', {
        path: screenshotPath,
        thumbnailPath,
      })

      return { screenshotPath, thumbnailPath }
    } catch (error) {
      taskLogger.error('Failed to capture screenshot', error as Error)
      throw error
    } finally {
      await page.close()
      taskLogger.debug('Closed page')
    }
  }

  /**
   * Capture preview screenshot if enabled in repository configuration (non-blocking)
   */
  async captureIterationScreenshotIfEnabled(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<void> {
    const taskLogger = this.serviceLogger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      // Get the task to access repository ID
      const task = await this.storage.getTask(taskId)
      if (!task) {
        taskLogger.warn('Task not found when checking screenshot setting')
        return
      }

      // Get the repository configuration
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        taskLogger.warn('Repository not found when checking screenshot setting')
        return
      }

      // Check if screenshots are enabled in the repository configuration
      if (!repository.previewConfig?.takeScreenshots) {
        taskLogger.debug('Screenshots disabled for repository, skipping capture')
        return
      }

      // Screenshots are enabled, proceed with capture
      await this.updateTaskWithScreenshot(taskId, iteration, task.agentType)
    } catch (error) {
      taskLogger.error('Failed to check screenshot setting or capture screenshot', error as Error)
      // Don't rethrow - this is non-blocking
    }
  }

  /**
   * Update task with screenshot after capturing (non-blocking)
   */
  private async updateTaskWithScreenshot(
    taskId: string,
    iteration: TaskIteration,
    agentType: string,
  ): Promise<void> {
    const taskLogger = this.serviceLogger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      const screenshotResult = await this.workspaceService.capturePreviewScreenshot(
        taskId,
        iteration.iterationNumber,
      )

      if (screenshotResult) {
        iteration.screenshotPath = screenshotResult.screenshotPath
        iteration.thumbnailPath = screenshotResult.thumbnailPath
        taskLogger.info('Screenshot and thumbnail paths added to iteration', {
          screenshotPath: screenshotResult.screenshotPath,
          thumbnailPath: screenshotResult.thumbnailPath,
        })

        // Save the updated iteration with screenshot and thumbnail paths
        const task = await this.storage.getTask(taskId)
        if (task) {
          // Find and update the iteration with screenshot paths
          const taskIteration = task.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (taskIteration) {
            taskIteration.screenshotPath = screenshotResult.screenshotPath
            taskIteration.thumbnailPath = screenshotResult.thumbnailPath
          }
          await this.storage.saveTask(task)
          taskLogger.debug('Task saved with screenshot and thumbnail paths')

          // Emit event for screenshot capture
          await this.eventService.emitEvent({
            taskId: task.taskId,
            type: 'taskIteration',
            subType: 'updated',
            agentType,
            iterationId: iteration.iterationNumber,
            payload: {
              status: taskIteration?.status || iteration.status,
              previousStatus: taskIteration?.status || iteration.status,
              iterationNumber: iteration.iterationNumber,
              screenshotPath: screenshotResult.screenshotPath,
              thumbnailPath: screenshotResult.thumbnailPath,
            },
            eventId: randomUUID(),
            timestamp: new Date(),
            isError: false,
          })
          taskLogger.info('Emitted screenshot update event', {
            screenshotPath: screenshotResult.screenshotPath,
            thumbnailPath: screenshotResult.thumbnailPath,
          })
        }
      } else {
        taskLogger.debug('No screenshot captured (server not configured or failed)')
      }
    } catch (error) {
      taskLogger.error('Failed to capture preview screenshot', error as Error)
      // Don't rethrow - this is non-blocking
    }
  }

  /**
   * Get iteration image (screenshot or thumbnail) for serving via HTTP
   */
  async getIterationImage(
    taskId: string,
    iterationNumber: number,
    imageType: 'screenshot' | 'thumbnail',
  ): Promise<Buffer> {
    const imageLogger = this.serviceLogger.child({ taskId, iterationNumber, imageType })

    // Get the task
    const task = await this.storage.getTask(taskId)
    if (!task) {
      imageLogger.warn('Task not found')
      const { HTTPException } = await import('hono/http-exception')
      throw new HTTPException(404, { message: `Task ${taskId} not found` })
    }

    // Find the iteration
    const iteration = task.iterations.find((it) => it.iterationNumber === iterationNumber)
    if (!iteration) {
      imageLogger.warn('Iteration not found')
      const { HTTPException } = await import('hono/http-exception')
      throw new HTTPException(404, {
        message: `Iteration ${iterationNumber} not found`,
      })
    }

    // Get the appropriate path
    const imagePath =
      imageType === 'screenshot' ? iteration.screenshotPath : iteration.thumbnailPath
    if (!imagePath) {
      imageLogger.warn(`No ${imageType} available for iteration`)
      const { HTTPException } = await import('hono/http-exception')
      throw new HTTPException(404, {
        message: `No ${imageType} available for this iteration`,
      })
    }

    // Read the image file
    try {
      const imageBuffer = await fs.readFile(imagePath)
      imageLogger.debug(`${imageType} served successfully`, { path: imagePath })
      return imageBuffer
    } catch (error) {
      imageLogger.error(`Failed to read ${imageType} file`, error as Error)
      const { HTTPException } = await import('hono/http-exception')
      throw new HTTPException(404, {
        message: `${imageType} file not found on disk`,
      })
    }
  }

  async cleanup(): Promise<void> {
    if (this.browser) {
      this.serviceLogger.debug('Closing Playwright browser')
      await this.browser.close()
      this.browser = undefined
    }
  }
}

// Singleton instance (for backward compatibility with workspace service)
let screenshotService: ScreenshotService | undefined

export function getScreenshotService(): ScreenshotService {
  if (!screenshotService) {
    throw new Error(
      'ScreenshotService not initialized. Call createScreenshotService first or use dependency injection.',
    )
  }
  return screenshotService
}

export function createScreenshotService(
  storage: StorageBackend,
  eventService: EventService,
  repositoryService: RepositoryService,
  workspaceService: WorkspaceService,
): ScreenshotService {
  if (!screenshotService) {
    screenshotService = new ScreenshotService(
      storage,
      eventService,
      repositoryService,
      workspaceService,
    )
  }
  return screenshotService
}

export async function cleanupScreenshotService(): Promise<void> {
  if (screenshotService) {
    await screenshotService.cleanup()
    screenshotService = undefined
  }
}
