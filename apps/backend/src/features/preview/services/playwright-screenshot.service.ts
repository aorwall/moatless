import type { TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { FileStorageService } from '../../file-storage/services/file-storage.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { PreviewOrchestratorService } from './preview-orchestrator.service'

/**
 * Service for capturing screenshots via sandbox browser.
 * Handles server orchestration, readiness checking, and screenshot capture.
 *
 * Uses the RemoteEnvironmentDriver's captureScreenshot method to capture
 * screenshots from the sandbox's browser, rather than running Playwright locally.
 */
@injectable()
export class PlaywrightScreenshotService {
  private serviceLogger = logger.child({ service: 'playwright-screenshot' })

  constructor(
    @inject(FileStorageService) private readonly fileStorage: FileStorageService,
    @inject(PreviewOrchestratorService)
    private readonly previewOrchestrator: PreviewOrchestratorService,
    @inject(RepositoryService)
    private readonly repositoryService: RepositoryService,
    @inject(TaskRepository)
    private readonly taskRepository: TaskRepository,
    @inject(WorkspaceRuntimeManager)
    private readonly workspaceRuntimeManager: WorkspaceRuntimeManager,
  ) {}

  /**
   * Capture screenshot if enabled in repository configuration.
   * Handles server orchestration, readiness checks, and screenshot capture.
   *
   * @param taskId - Task ID
   * @param iteration - Task iteration to capture screenshot for
   * @returns Object with screenshotFileId, or null if screenshots disabled or driver doesn't support screenshots
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
      // Get the task to access repository ID (use internal for background processing)
      const task = await this.taskRepository.getInternal(taskId)
      if (!task) {
        taskLogger.warn('Task not found when checking screenshot setting')
        return null
      }

      // Get the repository configuration (use internal for background processing)
      const repository = await this.repositoryService.getRepositoryByIdInternal(task.repositoryId)
      if (!repository) {
        taskLogger.warn('Repository not found when checking screenshot setting')
        return null
      }

      // Check if screenshots are enabled in the repository configuration
      if (!repository.previewConfig?.takeScreenshots) {
        taskLogger.debug('Screenshots disabled for repository, skipping capture')
        return null
      }

      // Get the workspace handle to access the driver
      const workspaceHandle = await this.workspaceRuntimeManager.getExistingWorkspaceHandle(taskId)
      if (!workspaceHandle) {
        taskLogger.warn('Workspace not found, skipping screenshot')
        return null
      }

      // Check if driver supports screenshot capture
      const { driver } = workspaceHandle
      if (!driver.captureScreenshot) {
        taskLogger.debug('Driver does not support screenshot capture, skipping')
        return null
      }

      // Check if server is running
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)
      if (serverStatus.status !== 'started') {
        taskLogger.info(
          { serverStatus: serverStatus.status },
          'Server not running, skipping screenshot',
        )
        return null
      }

      // Wait for server to be responsive using internal URL
      const internalUrl = await this.previewOrchestrator.getInternalUrl(taskId)
      if (internalUrl) {
        taskLogger.debug({ internalUrl }, 'Waiting for server to be ready')
        await this.waitForServerReady(
          internalUrl,
          (repository.previewConfig.startupTimeoutSeconds || 30) * 1000,
          taskId,
        )
      }

      // Capture screenshot via sandbox driver
      taskLogger.info('Capturing preview screenshot via sandbox')
      const screenshotResult = await driver.captureScreenshot()

      if (!screenshotResult.screenshotBase64) {
        throw new Error('Screenshot capture returned no base64 data')
      }

      // Convert base64 to Buffer
      const screenshotBuffer = Buffer.from(screenshotResult.screenshotBase64, 'base64')
      taskLogger.debug(
        {
          base64Length: screenshotResult.screenshotBase64.length,
          bufferSize: screenshotBuffer.length,
          url: screenshotResult.url,
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
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to capture screenshot')
      // Don't rethrow - this is non-blocking
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
}
