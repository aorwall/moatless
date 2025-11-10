import type { TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { FileStorageService } from '../../file-storage/services/file-storage.service'
import type { PlaywrightInteractionService } from './playwright-interaction.service'

/**
 * Service for capturing screenshots using PlaywrightInteractionService.
 * Integrates with existing Playwright page state for consistent browser management.
 */
@injectable()
export class PlaywrightScreenshotService {
  private serviceLogger = logger.child({ service: 'playwright-screenshot' })

  constructor(
    @inject('PlaywrightInteractionService')
    private readonly playwrightInteraction: PlaywrightInteractionService,
    @inject(FileStorageService) private readonly fileStorage: FileStorageService,
  ) {}

  /**
   * Capture screenshot for a task iteration using Playwright.
   * Reuses existing Playwright page if available, otherwise creates new page at "/".
   *
   * @param taskId - Task ID
   * @param iteration - Task iteration to capture screenshot for
   * @param serverUrl - Preview server URL (direct URL, not proxy)
   * @returns Object with screenshotFileId
   * @throws Error if screenshot capture fails
   */
  async captureIterationScreenshot(
    taskId: string,
    iteration: TaskIteration,
    serverUrl: string,
  ): Promise<{ screenshotFileId: string }> {
    const taskLogger = this.serviceLogger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      // Check if Playwright page already exists
      const currentUrl = this.playwrightInteraction.getCurrentUrl(taskId)

      if (!currentUrl) {
        // No existing page - create one by navigating to "/"
        taskLogger.info('No existing Playwright page, navigating to /', { serverUrl })
        await this.playwrightInteraction.navigateTo(taskId, '/', serverUrl)
      } else {
        // Page exists - use it at current location
        taskLogger.info('Reusing existing Playwright page', {
          currentUrl,
          note: 'Preserving agent browser state',
        })
      }

      // Capture screenshot with auto-restart capability
      taskLogger.info('Capturing screenshot via PlaywrightInteractionService')
      const snapshotResult = await this.playwrightInteraction.takeSnapshotWithAutoRestart(
        taskId,
        true, // includeScreenshot
      )

      if (!snapshotResult.screenshotBase64) {
        throw new Error('Screenshot capture returned no base64 data')
      }

      // Convert base64 to Buffer
      const screenshotBuffer = Buffer.from(snapshotResult.screenshotBase64, 'base64')
      taskLogger.debug('Screenshot captured', {
        base64Length: snapshotResult.screenshotBase64.length,
        bufferSize: screenshotBuffer.length,
      })

      // Upload to file storage
      const filename = `task-${taskId}-iter-${iteration.iterationNumber}.png`
      taskLogger.info('Uploading screenshot to file storage', { filename })

      const screenshotFile = await this.fileStorage.uploadFile(
        screenshotBuffer,
        filename,
        'image/png',
      )

      taskLogger.info('Screenshot uploaded successfully', {
        screenshotFileId: screenshotFile.fileId,
      })

      return {
        screenshotFileId: screenshotFile.fileId,
      }
    } catch (error) {
      taskLogger.error('Failed to capture screenshot', error as Error)
      throw error
    }
  }
}
