import type { TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { FileStorageService } from '../../file-storage/services/file-storage.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { PlaywrightInteractionService } from './playwright-interaction.service'
import { PreviewOrchestratorService } from './preview-orchestrator.service'

/**
 * Service for capturing screenshots using PlaywrightInteractionService.
 * Handles server orchestration, readiness checking, and screenshot capture.
 */
@injectable()
export class PlaywrightScreenshotService {
  private serviceLogger = logger.child({ service: 'playwright-screenshot' })

  constructor(
    @inject(PlaywrightInteractionService)
    private readonly playwrightInteraction: PlaywrightInteractionService,
    @inject(FileStorageService) private readonly fileStorage: FileStorageService,
    @inject(PreviewOrchestratorService)
    private readonly previewOrchestrator: PreviewOrchestratorService,
    @inject(RepositoryService)
    private readonly repositoryService: RepositoryService,
    @inject(TaskRepository)
    private readonly taskRepository: TaskRepository,
  ) {}

  /**
   * Capture screenshot if enabled in repository configuration.
   * Handles server orchestration, readiness checks, and screenshot capture.
   *
   * @param taskId - Task ID
   * @param iteration - Task iteration to capture screenshot for
   * @returns Object with screenshotFileId, or null if screenshots disabled
   */
  async captureIterationScreenshotIfEnabled(
    taskId: string,
    iteration: TaskIteration,
  ): Promise<{ screenshotFileId: string } | null> {
    const taskLogger = this.serviceLogger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      // Get the task to access repository ID
      const task = await this.taskRepository.get(taskId)
      if (!task) {
        taskLogger.warn('Task not found when checking screenshot setting')
        return null
      }

      // Get the repository configuration
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        taskLogger.warn('Repository not found when checking screenshot setting')
        return null
      }

      // Check if screenshots are enabled in the repository configuration
      if (!repository.previewConfig?.takeScreenshots) {
        taskLogger.debug('Screenshots disabled for repository, skipping capture')
        return null
      }

      // Check if server is running and get internal URL
      const internalUrl = await this.ensureServerRunning(taskId)
      if (!internalUrl) {
        taskLogger.info('Server not running, skipping screenshot')
        return null
      }

      // Wait for server to be responsive
      taskLogger.debug({ internalUrl }, 'Waiting for server to be ready')
      await this.waitForServerReady(
        internalUrl,
        (repository.previewConfig.startupTimeoutSeconds || 30) * 1000,
        taskId, // Pass taskId for cancellation checks
      )

      // Capture screenshot
      taskLogger.info({ internalUrl }, 'Capturing preview screenshot')
      return await this.captureScreenshotInternal(taskId, iteration, internalUrl)
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to capture screenshot')
      // Don't rethrow - this is non-blocking
      return null
    }
  }

  /**
   * Check if preview server is running and return internal URL.
   * Does not auto-start or restart the server.
   *
   * @param taskId - Task ID
   * @returns Internal server URL if running, or null if server not started
   */
  private async ensureServerRunning(taskId: string): Promise<string | null> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Get status
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)

      if (serverStatus.status !== 'started') {
        taskLogger.info('Server not started, skipping screenshot (server must be manually started)')
        return null
      }
      // Use running server as-is
      taskLogger.debug('Server already running, using for screenshot')

      if (serverStatus.status !== 'started' || !serverStatus.url) {
        taskLogger.warn('Server status verification failed')
        return null
      }

      // Get internal URL for Playwright (bypasses authentication)
      const internalUrl = await this.previewOrchestrator.getInternalUrl(taskId)
      if (!internalUrl) {
        taskLogger.warn('No internal URL available')
        return null
      }

      taskLogger.debug(
        {
          internalUrl,
          externalUrl: serverStatus.url,
        },
        'Server is running',
      )

      return internalUrl
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to ensure server running')
      return null
    }
  }

  /**
   * Wait for preview server to be ready by polling with HEAD requests.
   *
   * @param url - Server URL to check
   * @param timeout - Timeout in milliseconds
   * @param taskId - Optional task ID to check if server was stopped
   * @throws Error if server doesn't become ready within timeout
   */
  private async waitForServerReady(url: string, timeout = 30000, taskId?: string): Promise<void> {
    const startTime = Date.now()

    while (Date.now() - startTime < timeout) {
      // Check if server was stopped (cancellation check)
      if (taskId) {
        try {
          const status = await this.previewOrchestrator.getStatus(taskId)
          if (status.status === 'stopped') {
            this.serviceLogger.debug({ taskId }, 'Server was stopped during health check')
            return // Exit gracefully instead of throwing
          }
        } catch (_error) {
          // Task might have been deleted - exit gracefully
          this.serviceLogger.debug({ taskId }, 'Task deleted during health check')
          return
        }
      }

      try {
        const response = await fetch(url, {
          method: 'HEAD',
          signal: AbortSignal.timeout(5000),
        })

        if (response.ok || response.status < 500) {
          // Server is responding
          this.serviceLogger.debug({ url, status: response.status }, 'Server is ready')
          return
        }

        this.serviceLogger.debug(
          {
            url,
            status: response.status,
          },
          'Server responded with error status',
        )
      } catch (error) {
        this.serviceLogger.debug(
          {
            url,
            error: error instanceof Error ? error.message : String(error),
          },
          'Server not ready yet',
        )
      }

      // Wait 1 second before retry
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    throw new Error(`Server at ${url} did not become ready within ${timeout}ms`)
  }

  /**
   * Internal method to capture screenshot using Playwright.
   * Reuses existing Playwright page if available, otherwise creates new page at "/".
   *
   * @param taskId - Task ID
   * @param iteration - Task iteration to capture screenshot for
   * @param serverUrl - Preview server URL (direct URL, not proxy)
   * @returns Object with screenshotFileId
   * @throws Error if screenshot capture fails
   */
  private async captureScreenshotInternal(
    taskId: string,
    iteration: TaskIteration,
    serverUrl: string,
  ): Promise<{ screenshotFileId: string }> {
    const taskLogger = this.serviceLogger.child({
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    // Check if Playwright page already exists
    const currentUrl = this.playwrightInteraction.getCurrentUrl(taskId)

    if (!currentUrl) {
      // No existing page - create one by navigating to "/"
      taskLogger.info({ serverUrl }, 'No existing Playwright page, navigating to /')
      await this.playwrightInteraction.navigateTo(taskId, '/', serverUrl)
    } else {
      // Page exists - use it at current location
      taskLogger.info(
        {
          currentUrl,
          note: 'Preserving agent browser state',
        },
        'Reusing existing Playwright page',
      )
    }

    // Capture screenshot with auto-restart capability
    taskLogger.info('Capturing screenshot via PlaywrightInteractionService')
    const screenshotResult = await this.playwrightInteraction.takeScreenshotWithAutoRestart(taskId)

    if (!screenshotResult.screenshotBase64) {
      throw new Error('Screenshot capture returned no base64 data')
    }

    // Convert base64 to Buffer
    const screenshotBuffer = Buffer.from(screenshotResult.screenshotBase64, 'base64')
    taskLogger.debug(
      {
        base64Length: screenshotResult.screenshotBase64.length,
        bufferSize: screenshotBuffer.length,
      },
      'Screenshot captured',
    )

    // Upload to file storage
    const filename = `task-${taskId}-iter-${iteration.iterationNumber}.png`
    taskLogger.info({ filename }, 'Uploading screenshot to file storage')

    const screenshotFile = await this.fileStorage.uploadFile(
      screenshotBuffer,
      filename,
      'image/png',
    )

    taskLogger.info(
      {
        screenshotFileId: screenshotFile.fileId,
      },
      'Screenshot uploaded successfully',
    )

    return {
      screenshotFileId: screenshotFile.fileId,
    }
  }
}
