import { randomUUID } from 'node:crypto'
import type { TaskIteration } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/index'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import { PlaywrightScreenshotService } from './playwright-screenshot.service'
import { PreviewOrchestratorService } from './preview-orchestrator.service'

/**
 * Orchestrates screenshot capture operations between PreviewOrchestratorService and PlaywrightScreenshotService.
 * This service breaks the circular dependency by coordinating the workflow.
 */
@injectable()
export class ScreenshotOrchestrationService {
  private serviceLogger = logger.child({ service: 'screenshot-orchestration' })

  constructor(
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(EventService) private eventService: EventService,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(PreviewOrchestratorService) private previewOrchestrator: PreviewOrchestratorService,
    @inject(PlaywrightScreenshotService) private screenshotService: PlaywrightScreenshotService,
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
      // Get the task and repository to check configuration
      const task = await this.taskRepository.get(taskId)
      if (!task) {
        taskLogger.warn('Task not found for screenshot capture')
        return
      }

      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        taskLogger.warn('Repository not found for screenshot capture')
        return
      }

      // Check if server has dirty context (code changes made)
      const isDirty = await this.previewOrchestrator.runtime.isServerDirty(taskId)
      const autoReload = repository.previewConfig?.autoReload ?? false

      // Get status after start
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)

      if (serverStatus.status !== 'started') {
        await this.previewOrchestrator.start(taskId)
      } else if (isDirty && !autoReload) {
        // No auto-reload and server is dirty - restart will apply changes
        taskLogger.info('Code changes detected without auto-reload - restart will apply changes')
        await this.previewOrchestrator.restart(taskId)
      }

      if (serverStatus.status !== 'started' || !serverStatus.url) {
        taskLogger.warn('Server failed to start for screenshot')
        return
      }

      // Get internal URL for Playwright (bypasses authentication)
      const internalUrl = await this.previewOrchestrator.getInternalUrl(taskId)
      if (!internalUrl) {
        taskLogger.warn('No internal URL available for screenshot')
        return
      }

      // Wait for server to be responsive (use internal URL)
      // TODO: This should be handled in orchestrator
      taskLogger.debug('Waiting for server to be ready', {
        internalUrl,
        externalUrl: serverStatus.url,
      })
      await this.waitForServerReady(internalUrl, 30000) // 30 second timeout

      // Capture screenshot via PlaywrightScreenshotService (use internal URL)
      taskLogger.info('Capturing preview screenshot', {
        internalUrl,
        externalUrl: serverStatus.url,
      })
      const { screenshotFileId } = await this.screenshotService.captureIterationScreenshot(
        taskId,
        iteration,
        internalUrl,
      )

      if (screenshotFileId) {
        iteration.screenshotFileId = screenshotFileId
        taskLogger.info('Screenshot file ID added to iteration', {
          screenshotFileId,
        })

        // Save the updated iteration with screenshot file ID
        const task = await this.taskRepository.get(taskId)
        if (task) {
          // Find and update the iteration with screenshot file ID
          const taskIteration = task.iterations.find(
            (it) => it.iterationNumber === iteration.iterationNumber,
          )
          if (taskIteration) {
            taskIteration.screenshotFileId = screenshotFileId
          }
          await this.taskRepository.save(task)
          taskLogger.debug('Task saved with screenshot file ID')

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
              screenshotFileId,
            },
            eventId: randomUUID(),
            timestamp: new Date(),
            isError: false,
          })
          taskLogger.info('Emitted screenshot update event', {
            screenshotFileId,
          })
        }
      } else {
        taskLogger.debug('No screenshot captured')
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
