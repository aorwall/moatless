import { randomUUID } from 'node:crypto'
import type { TaskIteration } from '@moatless/types'
import { inject, singleton } from 'tsyringe'
import type { StorageBackend } from '../storage/index.js'
import { logger } from '../utils/logger.js'
import { EventService } from './event.service.js'
import { RepositoryService } from './repository.service.js'
import { WorkspaceService } from './workspace.service.js'

/**
 * Orchestrates screenshot capture operations between WorkspaceService and ScreenshotService.
 * This service breaks the circular dependency by coordinating the workflow.
 */
@singleton()
export class ScreenshotOrchestrationService {
  private serviceLogger = logger.child({ service: 'screenshot-orchestration' })

  constructor(
    @inject('Storage') private storage: StorageBackend,
    private eventService: EventService,
    private repositoryService: RepositoryService,
    private workspaceService: WorkspaceService,
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
      // Get preview server URL from workspace service
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
}
