import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { extractErrorMessage } from '../../preview/errors'
import { PlaywrightInteractionService } from '../../preview/services/playwright-interaction.service'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { RepositoryService } from '../../repositories/index'
import { TaskService } from '../../tasks/services/task.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'

interface McpToolResult {
  content: Array<
    | {
        type: 'text'
        text: string
      }
    | {
        type: 'image'
        data: string // base64 encoded image
        mimeType: string
      }
  >
  isError?: boolean
}

@injectable()
export class ToolService {
  constructor(
    @inject(TaskService) private taskFacade: TaskService,
    @inject(WorkspaceRuntimeManager) private runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewOrchestratorService) private previewOrchestrator: PreviewOrchestratorService,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(PlaywrightInteractionService) private playwrightService: PlaywrightInteractionService,
  ) {}

  /**
   * Handler for updateTaskMetadata tool
   */
  async handleUpdateTaskMetadata(
    taskId: string,
    args: { name?: string | undefined; description?: string | undefined },
  ): Promise<McpToolResult> {
    logger.info(
      {
        taskId,
        args,
      },
      'Tool execution: updateTaskMetadata',
    )

    const { name, description } = args

    if (!(name || description)) {
      return {
        content: [
          {
            type: 'text',
            text: 'Error: At least one field (name or description) must be provided',
          },
        ],
        isError: true,
      }
    }

    try {
      // Use TaskService.updateTask() which handles vibe repo renaming and event emission
      const updates: { name?: string; description?: string } = {}
      if (name !== undefined) {
        updates.name = name
      }
      if (description !== undefined) {
        updates.description = description
      }

      await this.taskFacade.updateTaskWithEvent(taskId, updates)

      const updatedFields = []
      if (name !== undefined) {
        updatedFields.push('name')
      }
      if (description !== undefined) {
        updatedFields.push('description')
      }

      return {
        content: [
          {
            type: 'text',
            text: `Successfully updated task metadata: ${updatedFields.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to update task metadata')

      // Check if it's a duplicate name error
      if (error && typeof error === 'object' && 'code' in error) {
        const moatlessError = error as { code: string; message: string }
        if (moatlessError.code === 'REPOSITORY_NAME_CONFLICT') {
          return {
            content: [
              {
                type: 'text',
                text: `Failed to update task: ${moatlessError.message}`,
              },
            ],
            isError: true,
          }
        }
      }

      return {
        content: [
          {
            type: 'text',
            text: `Failed to update task metadata: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for install tool
   */
  async handleInstall(taskId: string): Promise<McpToolResult> {
    logger.info({ taskId }, 'Tool execution: install')

    try {
      const handle = await this.runtimeManager.ensureWorkspace(taskId)

      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      const setupCommands = repository.codeExecution.setupCommands || []
      const needsSetup = handle.preview.needsSetup

      if (setupCommands.length === 0) {
        return {
          content: [
            {
              type: 'text',
              text: 'No setup commands configured for this repository',
            },
          ],
        }
      }

      if (needsSetup) {
        return {
          content: [
            {
              type: 'text',
              text: `Setup commands (${setupCommands.length}) will run automatically on first preview start: ${setupCommands.join(', ')}`,
            },
          ],
        }
      }

      return {
        content: [
          {
            type: 'text',
            text: `Setup commands have already been executed. Commands: ${setupCommands.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Installation query failed')
      return {
        content: [
          {
            type: 'text',
            text: `Installation failed: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for startOrRestartPreviewServer tool
   */
  async handleStartOrRestartPreviewServer(
    taskId: string,
    args: { force?: boolean } = {},
  ): Promise<McpToolResult> {
    const { force = false } = args
    logger.info({ taskId, force }, 'Tool execution: startOrRestartPreviewServer')

    try {
      // Ensure workspace is provisioned
      await this.runtimeManager.ensureWorkspace(taskId)

      // Delegate to orchestrator with smart restart logic
      const result = await this.previewOrchestrator.startOrRestart(taskId, undefined, force)

      // If restart was skipped, return the skip message
      if (result.skipMessage) {
        return {
          content: [
            {
              type: 'text',
              text: result.skipMessage,
            },
          ],
        }
      }

      // Return success with server status
      return {
        content: [
          {
            type: 'text',
            text: `Preview server started. Status: ${result.status.status}${
              result.status.url ? `, URL: ${result.status.url}` : ''
            }`,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to start preview server')
      return {
        content: [
          {
            type: 'text',
            text: `Failed to start preview server: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getPreviewServerLogs tool
   */
  async handleGetPreviewServerLogs(
    taskId: string,
    args: { lines?: number },
  ): Promise<McpToolResult> {
    logger.info(
      {
        taskId,
        args,
      },
      'Tool execution: getPreviewServerLogs',
    )

    try {
      const { lines = 100 } = args
      const logs = await this.previewOrchestrator.getLogs(taskId, lines)

      return {
        content: [
          {
            type: 'text',
            text: logs || 'No logs available',
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to get preview server logs')
      return {
        content: [
          {
            type: 'text',
            text: `Failed to get preview server logs: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for updateRepositoryConfig tool
   */
  async handleUpdateRepositoryConfig(
    taskId: string,
    args: {
      setupCommands?: string[]
      envVars?: Record<string, string>
      serverStartCommands?: string[]
    },
  ): Promise<McpToolResult> {
    logger.info(
      {
        taskId,
        args,
      },
      'Tool execution: updateRepositoryConfig',
    )

    const { setupCommands, envVars, serverStartCommands } = args

    if (!(setupCommands || envVars || serverStartCommands)) {
      return {
        content: [
          {
            type: 'text',
            text: 'Error: At least one field (setupCommands, envVars, or serverStartCommands) must be provided',
          },
        ],
        isError: true,
      }
    }

    try {
      // Get task to find repository ID
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      // Build updates object
      const updates = this.buildRepositoryConfigUpdates(
        repository,
        setupCommands,
        envVars,
        serverStartCommands,
      )

      // Update repository
      await this.repositoryService.updateRepository(task.repositoryId, updates)

      // Get updated field names for success message
      const updatedFields = this.getUpdatedFieldNames(setupCommands, envVars, serverStartCommands)

      return {
        content: [
          {
            type: 'text',
            text: `Successfully updated repository configuration: ${updatedFields.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to update repository config')
      return {
        content: [
          {
            type: 'text',
            text: `Failed to update repository configuration: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getRepositoryConfig tool
   */
  async handleGetRepositoryConfig(taskId: string): Promise<McpToolResult> {
    logger.info({ taskId }, 'Tool execution: getRepositoryConfig')

    try {
      // Get task to find repository ID
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      const config = {
        name: repository.name,
        kind: repository.kind,
        remoteUrl: repository.remote.url,
        setupCommands: repository.codeExecution.setupCommands || [],
        envVars: repository.codeExecution.envVars || {},
        serverStartCommands: repository.previewConfig?.serverStartCommands || [],
      }

      return {
        content: [
          {
            type: 'text',
            text: `Repository Configuration:
- Name: ${config.name}
- Kind: ${config.kind}
- Remote URL: ${config.remoteUrl}
- Setup Commands: ${config.setupCommands.length > 0 ? config.setupCommands.join(', ') : 'None'}
- Environment Variables: ${Object.keys(config.envVars).length > 0 ? Object.keys(config.envVars).join(', ') : 'None'}
- Server Start Commands: ${config.serverStartCommands.length > 0 ? config.serverStartCommands.join(', ') : 'None'}`,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to get repository config')
      return {
        content: [
          {
            type: 'text',
            text: `Failed to get repository configuration: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for navigatePreview tool
   */
  async handleNavigatePreview(taskId: string, args: { path: string }): Promise<McpToolResult> {
    logger.info({ taskId, path: args.path }, 'Tool execution: navigatePreview')

    try {
      // Get internal server URL (bypasses authentication) for Playwright
      // The public API only exposes the external URL (proxy with auth)
      // Note: Server status verification is now handled by playwrightService.navigateTo()
      const internalServerUrl = await this.previewOrchestrator.getInternalUrl(taskId)
      if (!internalServerUrl) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Preview server is running but internal URL not available',
            },
          ],
          isError: true,
        }
      }

      logger.debug(
        {
          taskId,
          internalServerUrl,
        },
        'Using internal server URL for Playwright navigation',
      )

      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.navigateTo(taskId, args.path, internalServerUrl)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to navigate preview')
      const errorMessage = extractErrorMessage(error, `Failed to navigate to ${args.path}`)
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for clickElement tool
   */
  async handleClickElement(taskId: string, args: { selector: string }): Promise<McpToolResult> {
    logger.info({ taskId, selector: args.selector }, 'Tool execution: clickElement')

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.clickElement(taskId, args.selector)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to click element')
      const errorMessage = extractErrorMessage(error, `Failed to click element "${args.selector}"`)
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for evaluateJavaScript tool
   */
  async handleEvaluateJavaScript(taskId: string, args: { script: string }): Promise<McpToolResult> {
    logger.info({ taskId, scriptLength: args.script.length }, 'Tool execution: evaluateJavaScript')

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.evaluateJavaScript(taskId, args.script)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to evaluate JavaScript')
      const errorMessage = extractErrorMessage(error, 'Failed to evaluate JavaScript')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for fillPreviewForm tool
   */
  async handleFillPreviewForm(
    taskId: string,
    args: { fields: Record<string, string> },
  ): Promise<McpToolResult> {
    logger.info(
      {
        taskId,
        fieldCount: Object.keys(args.fields).length,
      },
      'Tool execution: fillPreviewForm',
    )

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.fillForm(taskId, args.fields)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to fill form')
      const errorMessage = extractErrorMessage(error, 'Failed to fill form')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for takePreviewSnapshot tool
   * Captures accessibility snapshot of the current preview page (page structure/state)
   */
  async handleTakePreviewSnapshot(taskId: string): Promise<McpToolResult> {
    logger.info({ taskId }, 'Tool execution: takePreviewSnapshot')

    try {
      // Use auto-restart version which handles dirty context checking and restart logic
      // Note: takeSnapshotWithAutoRestart fetches repository config internally
      const result = await this.playwrightService.takeSnapshotWithAutoRestart(taskId)

      // Return the accessibility snapshot as text
      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to take preview snapshot')
      const errorMessage = extractErrorMessage(error, 'Failed to take snapshot')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for takePreviewScreenshot tool
   * Captures a visual screenshot of the current preview page
   */
  async handleTakePreviewScreenshot(taskId: string): Promise<McpToolResult> {
    logger.info({ taskId }, 'Tool execution: takePreviewScreenshot')

    try {
      // Use auto-restart version which handles dirty context checking and restart logic
      // Note: takeScreenshotWithAutoRestart fetches repository config internally
      const result = await this.playwrightService.takeScreenshotWithAutoRestart(taskId)

      // Return the screenshot image
      // Note: screenshotBase64 is guaranteed to be present on success (failures throw exceptions)
      return {
        content: [
          {
            type: 'image',
            data: result.screenshotBase64,
            mimeType: 'image/png',
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to take preview screenshot')
      const errorMessage = extractErrorMessage(error, 'Failed to take screenshot')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for typeText tool
   * Type text into an input field
   */
  async handleTypeText(
    taskId: string,
    args: { selector: string; text: string; delay?: number; clear?: boolean },
  ): Promise<McpToolResult> {
    logger.info({ taskId, selector: args.selector }, 'Tool execution: typeText')

    try {
      // Only pass defined optional properties
      const options: { delay?: number; clear?: boolean } = {}
      if (args.delay !== undefined) {
        options.delay = args.delay
      }
      if (args.clear !== undefined) {
        options.clear = args.clear
      }
      const result = await this.playwrightService.typeText(
        taskId,
        args.selector,
        args.text,
        options,
      )

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to type text')
      const errorMessage = extractErrorMessage(error, 'Failed to type text')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for hoverElement tool
   * Hover over an element
   */
  async handleHoverElement(taskId: string, args: { selector: string }): Promise<McpToolResult> {
    logger.info({ taskId, selector: args.selector }, 'Tool execution: hoverElement')

    try {
      const result = await this.playwrightService.hoverElement(taskId, args.selector)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to hover element')
      const errorMessage = extractErrorMessage(error, 'Failed to hover element')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getConsoleLogs tool
   * Get console logs captured since last retrieval
   */
  async handleGetConsoleLogs(taskId: string): Promise<McpToolResult> {
    logger.info({ taskId }, 'Tool execution: getConsoleLogs')

    try {
      const result = await this.playwrightService.getConsoleLogs(taskId)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to get console logs')
      const errorMessage = extractErrorMessage(error, 'Failed to get console logs')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getNetworkRequests tool
   * Get network requests captured since last retrieval
   */
  async handleGetNetworkRequests(
    taskId: string,
    args: { urlPattern?: string } = {},
  ): Promise<McpToolResult> {
    logger.info({ taskId, urlPattern: args.urlPattern }, 'Tool execution: getNetworkRequests')

    try {
      const result = await this.playwrightService.getNetworkRequests(taskId, args.urlPattern)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to get network requests')
      const errorMessage = extractErrorMessage(error, 'Failed to get network requests')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for waitForElement tool
   * Wait for an element to appear or reach a specific state
   */
  async handleWaitForElement(
    taskId: string,
    args: {
      selector: string
      state?: 'attached' | 'detached' | 'visible' | 'hidden'
      timeout?: number
    },
  ): Promise<McpToolResult> {
    logger.info({ taskId, selector: args.selector }, 'Tool execution: waitForElement')

    try {
      // Only pass defined optional properties
      const options: {
        state?: 'attached' | 'detached' | 'visible' | 'hidden'
        timeout?: number
      } = {}
      if (args.state !== undefined) {
        options.state = args.state
      }
      if (args.timeout !== undefined) {
        options.timeout = args.timeout
      }
      const result = await this.playwrightService.waitForElement(taskId, args.selector, options)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to wait for element')
      const errorMessage = extractErrorMessage(error, 'Failed to wait for element')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Build repository config updates object
   */
  private buildRepositoryConfigUpdates(
    repository: {
      codeExecution: Record<string, unknown>
      previewConfig?: Record<string, unknown> | undefined
    },
    setupCommands?: string[],
    envVars?: Record<string, string>,
    serverStartCommands?: string[],
  ): Record<string, unknown> {
    const updates: Record<string, unknown> = {}

    if (setupCommands || envVars) {
      updates['codeExecution'] = {
        ...repository.codeExecution,
        ...(setupCommands && { setupCommands }),
        ...(envVars && { envVars }),
      }
    }

    if (serverStartCommands) {
      updates['previewConfig'] = {
        ...repository.previewConfig,
        serverStartCommands,
      }
    }

    return updates
  }

  /**
   * Get list of updated field names
   */
  private getUpdatedFieldNames(
    setupCommands?: string[],
    envVars?: Record<string, string>,
    serverStartCommands?: string[],
  ): string[] {
    const updatedFields = []
    if (setupCommands !== undefined) {
      updatedFields.push('setupCommands')
    }
    if (envVars !== undefined) {
      updatedFields.push('envVars')
    }
    if (serverStartCommands !== undefined) {
      updatedFields.push('serverStartCommands')
    }
    return updatedFields
  }
}
