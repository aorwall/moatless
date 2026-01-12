import { logger } from '../../../shared/logger'
import type { PreviewServer } from '../../preview'
import type { McpToolResult } from '../types/mcp.types'
import type { PlaywrightScreenshotService } from './playwright-screenshot.service'

/**
 * Preview tool service
 * Handles tools that execute locally in the sandbox (no backend HTTP calls needed)
 *
 * Tools:
 * - startOrRestartPreviewServer
 * - getPreviewServerLogs
 * - navigatePreview
 * - clickElement
 * - evaluateJavaScript
 * - fillPreviewForm
 * - takePreviewSnapshot
 * - takePreviewScreenshot
 * - typeText
 * - hoverElement
 * - getConsoleLogs
 * - getNetworkRequests
 * - waitForElement
 */
export class PreviewToolService {
  private readonly logger = logger.child({ service: 'preview-tool' })

  constructor(
    private readonly previewServer: PreviewServer,
    private readonly screenshotService?: PlaywrightScreenshotService,
  ) {}

  /**
   * Handler for startOrRestartPreviewServer tool
   */
  handleStartOrRestartPreviewServer(args: { force?: boolean } = {}): Promise<McpToolResult> {
    const { force = false } = args
    this.logger.debug({ force }, 'Handling startOrRestartPreviewServer tool')

    try {
      // TODO: Implement preview server start/restart logic
      // This will use the previewServer instance to start/restart the server
      // await this.previewServer.start() or restart() based on current state

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: Preview server start/restart not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, force }, 'Failed to start/restart preview server')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error starting/restarting preview server: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for getPreviewServerLogs tool
   */
  handleGetPreviewServerLogs(args: { lines?: number } = {}): Promise<McpToolResult> {
    const { lines = 100 } = args
    this.logger.debug({ lines }, 'Handling getPreviewServerLogs tool')

    try {
      // TODO: Implement log retrieval
      // const logs = await this.previewServer.getLogs(lines)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: Preview server logs not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, lines }, 'Failed to get preview server logs')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error getting preview server logs: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for navigatePreview tool
   */
  handleNavigatePreview(args: { path: string }): Promise<McpToolResult> {
    this.logger.debug({ path: args.path }, 'Handling navigatePreview tool')

    try {
      // TODO: Implement navigation using PlaywrightInteractionService
      // await this.playwrightService.navigate(args.path)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Navigation to ${args.path} not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, path: args.path }, 'Failed to navigate preview')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error navigating preview: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for clickElement tool
   */
  handleClickElement(args: { selector: string }): Promise<McpToolResult> {
    this.logger.debug({ selector: args.selector }, 'Handling clickElement tool')

    try {
      // TODO: Implement click using PlaywrightInteractionService
      // const screenshot = await this.playwrightService.click(args.selector)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Click on ${args.selector} not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, selector: args.selector }, 'Failed to click element')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error clicking element: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for evaluateJavaScript tool
   */
  handleEvaluateJavaScript(args: { script: string }): Promise<McpToolResult> {
    this.logger.debug({ scriptLength: args.script.length }, 'Handling evaluateJavaScript tool')

    try {
      // TODO: Implement JS evaluation using PlaywrightInteractionService
      // const result = await this.playwrightService.evaluate(args.script)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: JavaScript evaluation not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to evaluate JavaScript')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error evaluating JavaScript: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for fillPreviewForm tool
   */
  handleFillPreviewForm(args: { fields: Record<string, string> }): Promise<McpToolResult> {
    this.logger.debug(
      { fieldCount: Object.keys(args.fields).length },
      'Handling fillPreviewForm tool',
    )

    try {
      // TODO: Implement form filling using PlaywrightInteractionService
      // const screenshot = await this.playwrightService.fillForm(args.fields)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Form filling for ${Object.keys(args.fields).length} fields not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error(
        { err: error as Error, fieldCount: Object.keys(args.fields).length },
        'Failed to fill form',
      )

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error filling form: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for takePreviewSnapshot tool
   */
  handleTakePreviewSnapshot(): Promise<McpToolResult> {
    this.logger.debug('Handling takePreviewSnapshot tool')

    try {
      // TODO: Implement snapshot using PlaywrightInteractionService
      // const snapshot = await this.playwrightService.takeSnapshot()

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: Preview snapshot not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to take preview snapshot')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error taking preview snapshot: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for takePreviewScreenshot tool
   */
  async handleTakePreviewScreenshot(args: { url?: string } = {}): Promise<McpToolResult> {
    this.logger.debug({ url: args.url }, 'Handling takePreviewScreenshot tool')

    try {
      // Check if screenshot service is available
      if (!this.screenshotService) {
        return {
          content: [
            {
              type: 'text',
              text: 'Screenshot service not initialized',
            },
          ],
          isError: true,
        }
      }

      if (!this.screenshotService.isAvailable()) {
        const reason = this.screenshotService.getUnavailableReason()
        return {
          content: [
            {
              type: 'text',
              text: `Screenshot service unavailable: ${reason}`,
            },
          ],
          isError: true,
        }
      }

      // Determine URL to screenshot
      let targetUrl: string

      if (args.url) {
        // Use provided URL
        targetUrl = args.url
        this.logger.debug({ targetUrl }, 'Using provided URL for screenshot')
      } else if (this.previewServer) {
        // Get URL from PreviewServer
        const status = this.previewServer.getStatus()

        if (status.status !== 'started' || !status.internalUrl) {
          return {
            content: [
              {
                type: 'text',
                text: `Preview server is not running (status: ${status.status}). Please start the server first or provide a URL.`,
              },
            ],
            isError: true,
          }
        }

        targetUrl = status.internalUrl
        this.logger.debug({ targetUrl }, 'Using PreviewServer URL for screenshot')
      } else {
        return {
          content: [
            {
              type: 'text',
              text: 'No URL provided and PreviewServer not available. Please provide a URL or start the preview server.',
            },
          ],
          isError: true,
        }
      }

      // Take screenshot
      const result = await this.screenshotService.takeScreenshot(targetUrl)

      return {
        content: [
          {
            type: 'text',
            text: `Screenshot captured successfully from ${result.url}\nPage title: ${result.title}`,
          },
          {
            type: 'image',
            data: result.screenshotBase64,
            mimeType: 'image/png',
          },
        ],
        isError: false,
      }
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to take preview screenshot')

      return {
        content: [
          {
            type: 'text',
            text: `Error taking preview screenshot: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for typeText tool
   */
  handleTypeText(args: {
    selector: string
    text: string
    delay?: number
    clear?: boolean
  }): Promise<McpToolResult> {
    this.logger.debug(
      { selector: args.selector, textLength: args.text.length },
      'Handling typeText tool',
    )

    try {
      // TODO: Implement typing using PlaywrightInteractionService
      // await this.playwrightService.type(args.selector, args.text, { delay: args.delay, clear: args.clear })

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Typing into ${args.selector} not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, selector: args.selector }, 'Failed to type text')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error typing text: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for hoverElement tool
   */
  handleHoverElement(args: { selector: string }): Promise<McpToolResult> {
    this.logger.debug({ selector: args.selector }, 'Handling hoverElement tool')

    try {
      // TODO: Implement hover using PlaywrightInteractionService
      // await this.playwrightService.hover(args.selector)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Hovering over ${args.selector} not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error, selector: args.selector }, 'Failed to hover element')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error hovering element: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for getConsoleLogs tool
   */
  handleGetConsoleLogs(): Promise<McpToolResult> {
    this.logger.debug('Handling getConsoleLogs tool')

    try {
      // TODO: Implement console log retrieval using PlaywrightInteractionService
      // const logs = await this.playwrightService.getConsoleLogs()

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: Console logs not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to get console logs')

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error getting console logs: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for getNetworkRequests tool
   */
  handleGetNetworkRequests(args: { urlPattern?: string } = {}): Promise<McpToolResult> {
    this.logger.debug({ urlPattern: args.urlPattern }, 'Handling getNetworkRequests tool')

    try {
      // TODO: Implement network request retrieval using PlaywrightInteractionService
      // const requests = await this.playwrightService.getNetworkRequests(args.urlPattern)

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: 'TODO: Network requests not yet implemented in sandbox',
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error(
        { err: error as Error, urlPattern: args.urlPattern },
        'Failed to get network requests',
      )

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error getting network requests: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }

  /**
   * Handler for waitForElement tool
   */
  handleWaitForElement(args: {
    selector: string
    state?: 'attached' | 'detached' | 'visible' | 'hidden'
    timeout?: number
  }): Promise<McpToolResult> {
    this.logger.debug(
      { selector: args.selector, state: args.state },
      'Handling waitForElement tool',
    )

    try {
      // TODO: Implement wait using PlaywrightInteractionService
      // await this.playwrightService.waitForElement(args.selector, { state: args.state, timeout: args.timeout })

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `TODO: Waiting for ${args.selector} not yet implemented in sandbox`,
          },
        ],
        isError: false,
      })
    } catch (error) {
      this.logger.error(
        { err: error as Error, selector: args.selector },
        'Failed to wait for element',
      )

      return Promise.resolve({
        content: [
          {
            type: 'text',
            text: `Error waiting for element: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      })
    }
  }
}

/**
 * Factory function to create PreviewToolService instance
 */
export async function createPreviewToolService(
  previewServer: PreviewServer,
): Promise<PreviewToolService> {
  // Create screenshot service
  const { createPlaywrightScreenshotService } = await import('./playwright-screenshot.service')
  const screenshotService = await createPlaywrightScreenshotService()

  return new PreviewToolService(previewServer, screenshotService)
}
