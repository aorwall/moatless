import { randomUUID } from 'node:crypto'
import type { TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { EventService } from '../features/events/index'
import { RepositoryService } from '../features/repositories/index'
import { TaskRepository } from '../features/tasks/db/task.repository'
import { PreviewManager } from '../features/workspace/services/preview-manager.service'
import { logger } from '../utils/logger'
import { ScreenshotService } from './screenshot.service'

/**
 * Orchestrates screenshot capture operations between PreviewManager and ScreenshotService.
 * This service breaks the circular dependency by coordinating the workflow.
 */
@injectable()
export class ScreenshotOrchestrationService {
  private serviceLogger = logger.child({ service: 'screenshot-orchestration' })

  constructor(
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(EventService) private eventService: EventService,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(PreviewManager) private previewManager: PreviewManager,
    @inject(ScreenshotService) private screenshotService: ScreenshotService,
  ) {}

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
      const task = await this.taskRepository.get(taskId)
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
      // Restart preview server to ensure fresh state
      taskLogger.info('Restarting preview server for screenshot capture')
      await this.previewManager.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const serverStatus = await this.previewManager.start(taskId)

      if (serverStatus.status !== 'started' || !serverStatus.url) {
        taskLogger.warn('Server failed to start for screenshot')
        return
      }

      // Wait for server to be responsive
      taskLogger.debug('Waiting for server to be ready', {
        url: serverStatus.url,
      })
      await this.waitForServerReady(serverStatus.url, 30000) // 30 second timeout

      // Capture screenshot
      taskLogger.info('Capturing preview screenshot', {
        url: serverStatus.url,
      })
      const screenshotResult = await this.screenshotService.captureScreenshot(
        serverStatus.url,
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
        const task = await this.taskRepository.get(taskId)
        if (task) {
          // Find and update the iteration with screenshot paths
          const taskIteration = task.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (taskIteration) {
            taskIteration.screenshotPath = screenshotResult.screenshotPath
            taskIteration.thumbnailPath = screenshotResult.thumbnailPath
          }
          await this.taskRepository.save(task)
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
   * Wait for preview server to be ready
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
}
