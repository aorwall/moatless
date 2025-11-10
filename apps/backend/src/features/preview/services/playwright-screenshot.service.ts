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

      // Ensure server is running and get internal URL
      const internalUrl = await this.ensureServerRunning(taskId, repository)
      if (!internalUrl) {
        taskLogger.warn('Failed to start server or get internal URL')
        return null
      }

      // Wait for server to be responsive
      taskLogger.debug('Waiting for server to be ready', { internalUrl })
      await this.waitForServerReady(
        internalUrl,
        repository.previewConfig.startupTimeoutSeconds || 3000 * 1000,
      )

      // Capture screenshot
      taskLogger.info('Capturing preview screenshot', { internalUrl })
      return await this.captureScreenshotInternal(taskId, iteration, internalUrl)
    } catch (error) {
      taskLogger.error('Failed to capture screenshot', error as Error)
      // Don't rethrow - this is non-blocking
      return null
    }
  }

  /**
   * Ensure preview server is running and return internal URL.
   * Starts server if not running, restarts if dirty context detected.
   *
   * @param taskId - Task ID
   * @param repository - Repository configuration
   * @returns Internal server URL, or null if server failed to start
   */
  private async ensureServerRunning(
    taskId: string,
    repository: { previewConfig?: { autoReload?: boolean | undefined } | undefined },
  ): Promise<string | null> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Check if server has dirty context (code changes made)
      const isDirty = await this.previewOrchestrator.runtime.isServerDirty(taskId)
      const autoReload = repository.previewConfig?.autoReload ?? false

      // Get status
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)

      if (serverStatus.status !== 'started') {
        taskLogger.info('Server not started, starting now')
        await this.previewOrchestrator.start(taskId)
      } else if (isDirty && !autoReload) {
        // No auto-reload and server is dirty - restart will apply changes
        taskLogger.info('Code changes detected without auto-reload - restart will apply changes')
        await this.previewOrchestrator.restart(taskId)
      }

      // Get status again after potential start/restart
      const updatedStatus = await this.previewOrchestrator.getStatus(taskId)

      if (updatedStatus.status !== 'started' || !updatedStatus.url) {
        taskLogger.warn('Server failed to start')
        return null
      }

      // Get internal URL for Playwright (bypasses authentication)
      const internalUrl = await this.previewOrchestrator.getInternalUrl(taskId)
      if (!internalUrl) {
        taskLogger.warn('No internal URL available')
        return null
      }

      taskLogger.debug('Server is running', {
        internalUrl,
        externalUrl: updatedStatus.url,
      })

      return internalUrl
    } catch (error) {
      taskLogger.error('Failed to ensure server running', error as Error)
      return null
    }
  }

  /**
   * Wait for preview server to be ready by polling with HEAD requests.
   *
   * @param url - Server URL to check
   * @param timeout - Timeout in milliseconds
   * @throws Error if server doesn't become ready within timeout
   */
  private async waitForServerReady(url: string, timeout = 30000): Promise<void> {
    const startTime = Date.now()

    while (Date.now() - startTime < timeout) {
      try {
        const response = await fetch(url, {
          method: 'HEAD',
          signal: AbortSignal.timeout(5000),
        })

        if (response.ok || response.status < 500) {
          // Server is responding
          this.serviceLogger.debug('Server is ready', { url, status: response.status })
          return
        }

        this.serviceLogger.debug('Server responded with error status', {
          url,
          status: response.status,
        })
      } catch (error) {
        this.serviceLogger.debug('Server not ready yet', {
          url,
          error: error instanceof Error ? error.message : String(error),
        })
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
  }
}
