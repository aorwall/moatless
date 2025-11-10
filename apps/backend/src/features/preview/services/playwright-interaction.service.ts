import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { type Browser, chromium, type Page } from '@playwright/test'
import { inject, injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import type { RepositoryService } from '../../repositories/index.js'
import { TaskRepository } from '../../tasks/db/task.repository.js'
import { PlaywrightError } from '../errors.js'
import type { PreviewServerLifecycleSink } from '../types/index.js'
import type {
  ClickResult,
  EvaluationResult,
  FormFillResult,
  NavigationResult,
  SnapshotResult,
} from '../types/playwright-results.js'
import type { PreviewOrchestratorService } from './preview-orchestrator.service.js'

export type PlaywrightMode = 'idle' | 'user' | 'agent'

interface PlaywrightPageState {
  page: Page
  browser: Browser
  mode: PlaywrightMode
  currentUrl: string
  createdAt: Date
}

/**
 * Service for managing Playwright browser interactions for preview servers.
 * Maintains persistent browser/page state per task for rich agent interactions.
 *
 * Inspired by Playwright MCP's Context pattern:
 * - Lazy browser/page initialization
 * - Persistent state across tool calls
 * - Mode-based control (agent vs user)
 * - Automatic cleanup on disposal
 */
@injectable()
export class PlaywrightInteractionService {
  private pageStates = new Map<string, PlaywrightPageState>()
  private serviceLogger = logger.child({ service: 'playwright-interaction' })
  private lifecycleSink: PreviewServerLifecycleSink | undefined
  private previewOrchestrator: PreviewOrchestratorService | undefined
  private repositoryService: RepositoryService | undefined

  constructor(@inject(TaskRepository) private readonly taskRepository: TaskRepository) {}

  /**
   * Set lifecycle sink for emitting events
   * Called by consumers that want to receive Playwright events
   */
  setLifecycleSink(sink: PreviewServerLifecycleSink): void {
    this.lifecycleSink = sink
  }

  /**
   * Set preview orchestrator for server status checks
   * Called by PreviewOrchestratorService to enable server verification
   */
  setPreviewOrchestrator(orchestrator: PreviewOrchestratorService): void {
    this.previewOrchestrator = orchestrator
  }

  /**
   * Set repository service for repository config lookups
   * Called by PreviewOrchestratorService to enable repository config fetching
   */
  setRepositoryService(repositoryService: RepositoryService): void {
    this.repositoryService = repositoryService
  }

  /**
   * Verify preview server is running before Playwright operations
   * Throws error if server is not started or has no URL
   */
  private async verifyServerRunning(taskId: string): Promise<void> {
    if (!this.previewOrchestrator) {
      // If orchestrator not set, skip verification (shouldn't happen in normal flow)
      this.serviceLogger.warn('Preview orchestrator not set, skipping server verification', {
        taskId,
      })
      return
    }

    const serverStatus = await this.previewOrchestrator.getStatus(taskId)

    if (serverStatus.status !== 'started' || !serverStatus.url) {
      throw new Error(
        `Preview server is not running. Status: ${serverStatus.status}. Please start the server before using Playwright tools.`,
      )
    }
  }

  /**
   * Ensure Playwright page exists for task, creating it if needed
   */
  async ensurePlaywrightPage(taskId: string, initialUrl: string): Promise<Page> {
    const taskLogger = this.serviceLogger.child({ taskId })

    let state = this.pageStates.get(taskId)

    if (state) {
      // Verify page is still connected
      if (!state.page.isClosed()) {
        taskLogger.debug('Reusing existing Playwright page', { mode: state.mode })
        return state.page
      }

      // Page was closed, clean up and recreate
      taskLogger.warn('Playwright page was closed, recreating')
      await this.cleanupPageState(taskId)
      state = undefined
    }

    // Create new browser and page
    taskLogger.info('Creating new Playwright browser and page', {
      initialUrl,
      note: 'Using direct server URL (not proxy) to bypass authentication',
    })

    const browser = await chromium.launch({
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    })

    const page = await browser.newPage()

    // Set viewport to match preview iframe
    await page.setViewportSize({ width: 1024, height: 720 })

    // Navigate to initial URL
    try {
      await page.goto(initialUrl, {
        waitUntil: 'networkidle',
        timeout: 30000,
      })
    } catch (error) {
      taskLogger.error('Initial navigation failed, closing browser', {
        error: error instanceof Error ? error.message : String(error),
        initialUrl,
      })
      // Clean up browser before throwing
      await browser.close()
      throw new Error(
        `Failed to navigate to ${initialUrl}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }

    state = {
      page,
      browser,
      mode: 'idle',
      currentUrl: page.url(),
      createdAt: new Date(),
    }

    this.pageStates.set(taskId, state)

    taskLogger.info('Playwright page created successfully', {
      url: state.currentUrl,
      mode: state.mode,
    })

    return page
  }

  /**
   * Navigate Playwright page to a relative path
   * @param taskId - Task ID
   * @param path - Relative path to navigate to (e.g., "/", "/#/tasks", "/about")
   * @param serverUrl - Preview server base URL
   * @throws PlaywrightError if navigation fails or path is invalid
   */
  async navigateTo(taskId: string, path: string, serverUrl: string): Promise<NavigationResult> {
    const taskLogger = this.serviceLogger.child({ taskId, path })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      // IMPORTANT: serverUrl should be the DIRECT server URL (not proxy URL)
      // The direct server URL bypasses authentication, while proxy URL has auth
      taskLogger.debug('Playwright navigate with server URL', {
        serverUrl,
        note: 'Should be direct server URL, not proxy URL',
      })

      // Construct full URL (validates path is relative)
      const fullUrl = this.constructFullUrl(path, serverUrl)
      taskLogger.info('Navigating Playwright page', { path, fullUrl, serverUrl })

      // Ensure page exists
      const page = await this.ensurePlaywrightPage(taskId, fullUrl)

      // Set mode to agent
      await this.setMode(taskId, 'agent')

      // Navigate
      await page.goto(fullUrl, {
        waitUntil: 'networkidle',
        timeout: 30000,
      })

      // Wait for page to stabilize
      await page.waitForTimeout(2000)

      // Update current URL
      const state = this.pageStates.get(taskId)
      if (state) {
        state.currentUrl = page.url()
      }

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'navigate', page.url())

      taskLogger.info('Navigation completed successfully', {
        finalUrl: page.url(),
        title: await page.title(),
      })

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: pageUrl,
        title: pageTitle,
        snapshot,
        message: `Successfully navigated to: ${pageUrl}\nPage title: ${pageTitle}\n\nPage snapshot:\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error('Navigation failed', error as Error)
      throw new PlaywrightError(
        `Failed to navigate to ${path}`,
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Click element on Playwright page
   * @throws PlaywrightError if click fails or page not initialized
   */
  async clickElement(taskId: string, selector: string): Promise<ClickResult> {
    const taskLogger = this.serviceLogger.child({ taskId, selector })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info('Clicking element', { selector })

      // Set mode to agent
      await this.setMode(taskId, 'agent')

      const { page } = state

      // Click element
      await page.click(selector, { timeout: 10000 })

      // Wait for navigation/page updates
      await page.waitForTimeout(1000)

      // Update current URL
      state.currentUrl = page.url()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'click', page.url())

      taskLogger.info('Click completed successfully', {
        url: page.url(),
        title: await page.title(),
      })

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: pageUrl,
        title: pageTitle,
        snapshot,
        message: `Successfully clicked element: ${selector}\nCurrent URL: ${pageUrl}\nPage title: ${pageTitle}\n\nPage snapshot:\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error('Click failed', error as Error)
      throw new PlaywrightError(
        `Failed to click element: ${selector}`,
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Evaluate JavaScript in Playwright page context
   * @throws PlaywrightError if evaluation fails or page not initialized
   */
  async evaluateJavaScript(taskId: string, script: string): Promise<EvaluationResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info('Evaluating JavaScript', { scriptLength: script.length })

      // Set mode to agent
      await this.setMode(taskId, 'agent')

      const { page } = state

      // Evaluate script
      const result = await page.evaluate(script)

      // Wait for any side effects
      await page.waitForTimeout(500)

      // Update current URL in case script navigated
      state.currentUrl = page.url()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'evaluate', page.url())

      taskLogger.info('JavaScript evaluation completed', {
        resultType: typeof result,
        url: page.url(),
      })

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: pageUrl,
        title: pageTitle,
        result,
        snapshot,
        message: `Successfully evaluated JavaScript\nResult: ${JSON.stringify(result, null, 2)}\nCurrent URL: ${pageUrl}\nPage title: ${pageTitle}\n\nPage snapshot:\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error('JavaScript evaluation failed', error as Error)
      throw new PlaywrightError(
        'Failed to evaluate JavaScript',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Fill form fields on Playwright page
   * @throws PlaywrightError if form fill fails or page not initialized
   */
  async fillForm(taskId: string, fields: Record<string, string>): Promise<FormFillResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info('Filling form fields', { fieldCount: Object.keys(fields).length })

      // Set mode to agent
      await this.setMode(taskId, 'agent')

      const { page } = state

      // Fill each field
      for (const [selector, value] of Object.entries(fields)) {
        await page.fill(selector, value)
        await page.waitForTimeout(100) // Small delay between fields
      }

      // Wait for any validation/updates
      await page.waitForTimeout(500)

      // Update current URL
      state.currentUrl = page.url()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'fillForm', page.url())

      taskLogger.info('Form fill completed successfully', {
        url: page.url(),
        title: await page.title(),
      })

      const pageUrl = page.url()
      const pageTitle = await page.title()
      const fieldCount = Object.keys(fields).length

      return {
        url: pageUrl,
        title: pageTitle,
        snapshot,
        message: `Successfully filled ${fieldCount} form fields\nCurrent URL: ${pageUrl}\nPage title: ${pageTitle}\n\nPage snapshot:\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error('Form fill failed', error as Error)
      throw new PlaywrightError(
        'Failed to fill form fields',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Take screenshot of current Playwright page state
   * @throws PlaywrightError if snapshot fails or page not initialized
   */
  async takeSnapshot(taskId: string, includeScreenshot = true): Promise<SnapshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info('Taking page snapshot', { includeScreenshot })

      const { page } = state
      const pageUrl = page.url()
      const pageTitle = await page.title()

      // Capture screenshot if requested
      let screenshotBase64 = ''
      let screenshotPath: string | undefined

      if (includeScreenshot) {
        const screenshotResult = await this.captureScreenshot(taskId, page, taskLogger)
        screenshotBase64 = screenshotResult.screenshotBase64
        screenshotPath = screenshotResult.screenshotPath

        if (!screenshotBase64) {
          throw new Error('Screenshot capture failed - no base64 data returned')
        }
      }

      taskLogger.info('Snapshot captured successfully', {
        url: pageUrl,
        title: pageTitle,
        hasScreenshot: includeScreenshot,
      })

      // Build result with optional screenshotPath
      const result: SnapshotResult = {
        url: pageUrl,
        title: pageTitle,
        screenshotBase64,
      }

      if (screenshotPath !== undefined) {
        result.screenshotPath = screenshotPath
      }

      return result
    } catch (error) {
      taskLogger.error('Snapshot failed', error as Error)
      throw new PlaywrightError(
        'Failed to capture snapshot',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Take snapshot with auto-restart on dirty context
   * Checks if workspace code has changed and restarts server if needed (when autoReload is disabled)
   * Fetches repository config internally to determine auto-reload setting
   * @throws PlaywrightError if snapshot fails, page not initialized, or services not configured
   */
  async takeSnapshotWithAutoRestart(
    taskId: string,
    includeScreenshot = true,
  ): Promise<SnapshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify services are configured
      if (!this.repositoryService) {
        throw new Error(
          'RepositoryService must be configured before calling takeSnapshotWithAutoRestart',
        )
      }

      if (!this.previewOrchestrator) {
        throw new Error(
          'PreviewOrchestrator must be configured before calling takeSnapshotWithAutoRestart',
        )
      }

      // Check server status and auto-start if needed
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)

      if (serverStatus.status !== 'started') {
        taskLogger.info('Preview server not running, starting it', {
          currentStatus: serverStatus.status,
        })
        await this.previewOrchestrator.startAndWaitForReady(taskId)
        taskLogger.info('Preview server started successfully')
      }

      // Fetch task and repository to get auto-reload config
      const task = await this.taskRepository.get(taskId)
      if (!task) {
        throw new Error(`Task not found: ${taskId}`)
      }

      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      const autoReload = repository?.previewConfig?.autoReload ?? false

      taskLogger.debug('Repository config fetched', {
        repositoryId: task.repositoryId,
        autoReload,
      })

      // Check if server has dirty context (refresh status after potential start)
      const currentStatus = await this.previewOrchestrator.getStatus(taskId)
      const isDirty = currentStatus.dirtyContext ?? false

      if (isDirty && !autoReload) {
        // Code changes detected and no auto-reload - full restart needed
        taskLogger.info('Restarting preview server before snapshot (code changes detected)', {
          taskId,
        })

        await this.previewOrchestrator.stop(taskId)
        await new Promise((resolve) => setTimeout(resolve, 1000))
        await this.previewOrchestrator.startAndWaitForReady(taskId)

        taskLogger.info('Preview server restarted, taking snapshot', { taskId })
      } else if (isDirty && autoReload) {
        // Code changes but auto-reload enabled - skip restart
        taskLogger.info('Taking snapshot with dirty context (auto-reload enabled)', {
          taskId,
        })
      }

      // Take the snapshot
      return await this.takeSnapshot(taskId, includeScreenshot)
    } catch (error) {
      taskLogger.error('Snapshot with auto-restart failed', error as Error)
      throw new PlaywrightError(
        'Failed to capture snapshot with auto-restart',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Set Playwright mode for task
   */
  async setMode(taskId: string, mode: PlaywrightMode): Promise<void> {
    const state = this.pageStates.get(taskId)
    if (!state) {
      return
    }

    const previousMode = state.mode
    state.mode = mode

    this.serviceLogger.debug('Playwright mode changed', {
      taskId,
      previousMode,
      newMode: mode,
    })

    // Notify lifecycle sink of mode change
    if (this.lifecycleSink?.onPlaywrightModeChange) {
      await this.lifecycleSink.onPlaywrightModeChange(taskId, mode)
    }
  }

  /**
   * Get current mode for task
   */
  getMode(taskId: string): PlaywrightMode {
    const state = this.pageStates.get(taskId)
    return state?.mode ?? 'idle'
  }

  /**
   * Get current URL from Playwright page
   */
  getCurrentUrl(taskId: string): string | undefined {
    const state = this.pageStates.get(taskId)
    return state?.currentUrl
  }

  /**
   * Close Playwright page and browser for task
   */
  async closePlaywrightPage(taskId: string): Promise<void> {
    const taskLogger = this.serviceLogger.child({ taskId })
    taskLogger.info('Closing Playwright page')

    await this.cleanupPageState(taskId)

    taskLogger.info('Playwright page closed successfully')
  }

  /**
   * Cleanup all Playwright resources
   */
  async cleanup(): Promise<void> {
    this.serviceLogger.info('Cleaning up all Playwright resources', {
      activePages: this.pageStates.size,
    })

    const taskIds = Array.from(this.pageStates.keys())
    await Promise.all(taskIds.map((taskId) => this.cleanupPageState(taskId)))

    this.serviceLogger.info('Playwright cleanup completed')
  }

  /**
   * Clean up Playwright page state for a specific task
   */
  private async cleanupPageState(taskId: string): Promise<void> {
    const state = this.pageStates.get(taskId)
    if (!state) {
      return
    }

    try {
      if (!state.page.isClosed()) {
        await state.page.close()
      }
    } catch (error) {
      this.serviceLogger.warn('Error closing Playwright page', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
    }

    try {
      await state.browser.close()
    } catch (error) {
      this.serviceLogger.warn('Error closing Playwright browser', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
    }

    this.pageStates.delete(taskId)
  }

  /**
   * Validate that input is a relative path, not an absolute URL
   * @throws Error if path is invalid
   */
  private validateRelativePath(path: string): void {
    // Reject absolute URLs
    if (path.startsWith('http://') || path.startsWith('https://')) {
      throw new Error(
        'Absolute URLs are not allowed. Use relative paths only (e.g., "/", "/#/tasks", "/dashboard")',
      )
    }

    // Reject directory traversal attempts
    if (path.includes('..')) {
      throw new Error('Directory traversal is not allowed in paths')
    }

    // Ensure path starts with /
    if (!path.startsWith('/')) {
      throw new Error('Path must start with / (e.g., "/", "/#/tasks", "/about")')
    }
  }

  /**
   * Construct full URL from path and server URL
   */
  private constructFullUrl(pathInput: string, serverUrl: string): string {
    // Validate that input is a relative path
    this.validateRelativePath(pathInput)

    // Construct from server URL and path
    const baseUrl = serverUrl.endsWith('/') ? serverUrl.slice(0, -1) : serverUrl
    return `${baseUrl}${pathInput}`
  }

  /**
   * Capture screenshot from Playwright page
   */
  private async captureScreenshot(
    taskId: string,
    page: Page,
    taskLogger: ReturnType<typeof logger.child>,
  ): Promise<{ screenshotPath: string; screenshotBase64: string }> {
    // Prepare screenshot path
    const screenshotDir = path.join(getDataDir(), 'screenshots', taskId)
    await fs.mkdir(screenshotDir, { recursive: true })

    const filename = `playwright-${randomUUID()}.png`
    const screenshotPath = path.join(screenshotDir, filename)

    // Capture screenshot
    taskLogger.debug('Capturing screenshot', { filename })
    await page.screenshot({
      path: screenshotPath,
      fullPage: false,
      type: 'png',
    })

    // Read screenshot and convert to base64
    const imageBuffer = await fs.readFile(screenshotPath)
    const screenshotBase64 = imageBuffer.toString('base64')

    taskLogger.debug('Screenshot captured successfully', {
      path: screenshotPath,
      base64Length: screenshotBase64.length,
    })

    return { screenshotPath, screenshotBase64 }
  }

  /**
   * Capture accessibility snapshot from Playwright page
   * Returns ARIA tree as formatted text (similar to Playwright MCP)
   */
  private async captureAccessibilitySnapshot(
    page: Page,
    taskLogger: ReturnType<typeof logger.child>,
  ): Promise<string> {
    taskLogger.debug('Capturing accessibility snapshot')

    try {
      // Get accessibility snapshot from Playwright
      const snapshot = await page.accessibility.snapshot()

      if (!snapshot) {
        return 'No accessibility tree available'
      }

      // Format the snapshot as readable text
      const formatted = this.formatAccessibilityTree(snapshot, 0)

      taskLogger.debug('Accessibility snapshot captured successfully', {
        length: formatted.length,
      })

      return formatted
    } catch (error) {
      taskLogger.warn('Failed to capture accessibility snapshot', {
        error: error instanceof Error ? error.message : String(error),
      })
      return 'Failed to capture accessibility snapshot'
    }
  }

  /**
   * Format accessibility tree snapshot as indented text
   */
  private formatAccessibilityTree(node: unknown, depth: number): string {
    if (!node || typeof node !== 'object') {
      return ''
    }

    const n = node as {
      role?: string
      name?: string
      value?: string
      description?: string
      children?: unknown[]
    }

    const indent = '  '.repeat(depth)
    const parts: string[] = []

    // Format the node
    if (n.role) {
      let line = `${indent}${n.role}`
      if (n.name) {
        line += ` "${n.name}"`
      }
      if (n.value) {
        line += ` [${n.value}]`
      }
      if (n.description) {
        line += ` (${n.description})`
      }
      parts.push(line)
    }

    // Recursively format children
    if (n.children && Array.isArray(n.children)) {
      for (const child of n.children) {
        const childText = this.formatAccessibilityTree(child, depth + 1)
        if (childText) {
          parts.push(childText)
        }
      }
    }

    return parts.join('\n')
  }

  /**
   * Notify lifecycle sink of Playwright action
   */
  private async notifyPlaywrightAction(
    taskId: string,
    actionType: string,
    url: string,
  ): Promise<void> {
    if (!this.lifecycleSink?.onPlaywrightAction) {
      return
    }

    try {
      await this.lifecycleSink.onPlaywrightAction(taskId, {
        type: actionType,
        url,
        mode: this.getMode(taskId),
        timestamp: new Date(),
      })
    } catch (error) {
      this.serviceLogger.warn('Failed to notify Playwright action', {
        taskId,
        actionType,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
}
