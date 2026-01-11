import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Browser, BrowserType, Page } from '@playwright/test'
import { inject, injectable } from 'inversify'
import type { Logger } from 'pino'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import type { RepositoryService } from '../../repositories/index.js'
import { TaskRepository } from '../../tasks/db/task.repository.js'
import { PlaywrightError, PlaywrightUnavailableError } from '../errors.js'
import type { PreviewServerLifecycleSink } from '../types/index.js'
import type {
  ClickResult,
  ConsoleLogsResult,
  ConsoleMessage,
  EvaluationResult,
  FormFillResult,
  NavigationResult,
  NetworkRequest,
  NetworkRequestsResult,
  ScreenshotResult,
  SnapshotResult,
  WaitResult,
} from '../types/playwright-results.js'
import type { PreviewOrchestratorService } from './preview-orchestrator.service.js'

export type PlaywrightMode = 'idle' | 'user' | 'agent'

interface PlaywrightPageState {
  page: Page
  browser: Browser
  mode: PlaywrightMode
  currentUrl: string
  createdAt: Date
  consoleMessages: ConsoleMessage[]
  networkRequests: NetworkRequest[]
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
  /** Dynamically loaded chromium module (undefined if Playwright not installed) */
  private chromiumModule: BrowserType | undefined

  /** Whether Playwright browsers are installed and available */
  private playwrightAvailable = true // Assume available until checked
  /** Reason why Playwright is unavailable (if not available) */
  private playwrightUnavailableReason: string | undefined
  /** Whether availability has been checked */
  private availabilityChecked = false

  constructor(@inject(TaskRepository) private readonly taskRepository: TaskRepository) {}

  /**
   * Initialize the service by checking Playwright availability.
   * Should be called once during application startup.
   * Uses dynamic import so the server can start even without Playwright installed.
   */
  async initialize(): Promise<void> {
    if (this.availabilityChecked) {
      return
    }

    this.availabilityChecked = true

    try {
      // Dynamic import - fails gracefully if Playwright not installed
      const playwright = await import('@playwright/test')
      this.chromiumModule = playwright.chromium

      // Get the chromium executable path from Playwright
      const executablePath = this.chromiumModule.executablePath()

      // Check if the executable exists
      const exists = await fs
        .access(executablePath)
        .then(() => true)
        .catch(() => false)

      if (!exists) {
        this.playwrightAvailable = false
        this.playwrightUnavailableReason = `Chromium executable not found at ${executablePath}. Install with: bunx playwright install chromium`
        this.serviceLogger.warn(
          { executablePath },
          'Playwright chromium not available - browser interactions and screenshots will be disabled',
        )
      } else {
        this.serviceLogger.info({ executablePath }, 'Playwright chromium available')
      }
    } catch (error) {
      this.playwrightAvailable = false
      this.chromiumModule = undefined
      this.playwrightUnavailableReason = `Playwright not installed: ${error instanceof Error ? error.message : String(error)}`
      this.serviceLogger.warn(
        { error: error instanceof Error ? error.message : String(error) },
        'Playwright not available - browser interactions and screenshots will be disabled',
      )
    }
  }

  /**
   * Check if Playwright is available for browser interactions.
   * @returns true if Playwright browsers are installed and available
   */
  isAvailable(): boolean {
    return this.playwrightAvailable
  }

  /**
   * Get the reason why Playwright is unavailable.
   * @returns undefined if available, or a description of why it's unavailable
   */
  getUnavailableReason(): string | undefined {
    return this.playwrightUnavailableReason
  }

  /**
   * Assert that Playwright is available, throwing if not.
   * Use this at the start of methods that require Playwright.
   * @throws PlaywrightUnavailableError if Playwright is not available
   */
  private assertAvailable(): void {
    if (!this.playwrightAvailable) {
      throw new PlaywrightUnavailableError(
        this.playwrightUnavailableReason || 'Playwright browsers not installed',
      )
    }
  }

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
      this.serviceLogger.warn(
        {
          taskId,
        },
        'Preview orchestrator not set, skipping server verification',
      )
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
   * @throws PlaywrightUnavailableError if Playwright browsers are not installed
   */
  async ensurePlaywrightPage(taskId: string, initialUrl: string): Promise<Page> {
    const taskLogger = this.serviceLogger.child({ taskId })

    let state = this.pageStates.get(taskId)

    if (state) {
      // Verify page is still connected
      if (!state.page.isClosed()) {
        taskLogger.debug({ mode: state.mode }, 'Reusing existing Playwright page')
        return state.page
      }

      // Page was closed, clean up and recreate
      taskLogger.warn(
        {
          initialUrl,
        },
        'Playwright page was closed, recreating',
      )
      await this.cleanupPageState(taskId)
      state = undefined
    }

    // Check Playwright availability before creating browser
    this.assertAvailable()

    if (!this.chromiumModule) {
      throw new PlaywrightUnavailableError('Playwright not initialized - call initialize() first')
    }

    // Create new browser and page
    taskLogger.info(
      {
        initialUrl,
        note: 'Using direct server URL (not proxy) to bypass authentication',
      },
      'Creating new Playwright browser and page',
    )

    const browser = await this.chromiumModule.launch({
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    })

    const page = await browser.newPage()

    // Set viewport to match preview iframe
    await page.setViewportSize({ width: 1024, height: 720 })

    // Initialize console messages and network requests arrays
    const consoleMessages: ConsoleMessage[] = []
    const networkRequests: NetworkRequest[] = []

    // Setup console listener
    page.on('console', (msg) => {
      consoleMessages.push({
        type: msg.type() as ConsoleMessage['type'],
        text: msg.text(),
        timestamp: new Date(),
      })
    })

    // Setup network request listener
    page.on('request', (request) => {
      const startTime = Date.now()
      request.response().then(
        (response) => {
          const networkRequest: NetworkRequest = {
            url: request.url(),
            method: request.method(),
            timing: Date.now() - startTime,
            timestamp: new Date(),
          }
          const responseStatus = response?.status()
          if (responseStatus !== undefined) {
            networkRequest.status = responseStatus
          }
          networkRequests.push(networkRequest)
        },
        () => {
          // Request failed - don't include status property
          networkRequests.push({
            url: request.url(),
            method: request.method(),
            timing: Date.now() - startTime,
            timestamp: new Date(),
          })
        },
      )
    })

    // Navigate to initial URL
    try {
      await page.goto(initialUrl, {
        waitUntil: 'domcontentloaded',
        timeout: 30000,
      })
    } catch (error) {
      taskLogger.error('Initial navigation failed, closing browser')
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
      consoleMessages,
      networkRequests,
    }

    this.pageStates.set(taskId, state)

    taskLogger.info(
      {
        url: state.currentUrl,
        mode: state.mode,
      },
      'Playwright page created successfully',
    )

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
      taskLogger.debug(
        {
          serverUrl,
          note: 'Should be direct server URL, not proxy URL',
        },
        'Playwright navigate with server URL',
      )

      // Construct full URL (validates path is relative)
      const fullUrl = this.constructFullUrl(path, serverUrl)
      taskLogger.info({ path, fullUrl, serverUrl }, 'Navigating Playwright page')

      // Ensure page exists
      const page = await this.ensurePlaywrightPage(taskId, fullUrl)

      // Set mode to agent
      await this.setMode(taskId, 'agent')

      // Navigate
      await page.goto(fullUrl, {
        waitUntil: 'domcontentloaded',
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

      taskLogger.info(
        {
          finalUrl: page.url(),
          title: await page.title(),
        },
        'Navigation completed successfully',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Successfully navigated to the page.\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Navigation failed')
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

      taskLogger.info({ selector }, 'Clicking element')

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

      taskLogger.info(
        {
          url: page.url(),
          title: await page.title(),
        },
        'Click completed successfully',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Successfully clicked element: ${selector}\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Click failed')
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

      taskLogger.info({ scriptLength: script.length }, 'Evaluating JavaScript')

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

      taskLogger.info(
        {
          resultType: typeof result,
          url: page.url(),
        },
        'JavaScript evaluation completed',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        result,
        snapshot,
        message: `Successfully evaluated JavaScript\nResult: ${JSON.stringify(result, null, 2)}\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'JavaScript evaluation failed')
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

      taskLogger.info({ fieldCount: Object.keys(fields).length }, 'Filling form fields')

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

      taskLogger.info(
        {
          url: page.url(),
          title: await page.title(),
        },
        'Form fill completed successfully',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()
      const fieldCount = Object.keys(fields).length

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Successfully filled ${fieldCount} form fields\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Form fill failed')
      throw new PlaywrightError(
        'Failed to fill form fields',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Take accessibility snapshot of current Playwright page state
   * Use this to understand page state/structure without visual representation.
   * For visual appearance, use takeScreenshot() instead.
   * @throws PlaywrightError if snapshot fails or page not initialized
   */
  async takeSnapshot(taskId: string): Promise<SnapshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      // Auto-initialize page if it doesn't exist
      const currentUrl = this.getCurrentUrl(taskId)
      if (!currentUrl) {
        taskLogger.info('No existing Playwright page, auto-initializing to /')
        if (!this.previewOrchestrator) {
          throw new Error('Preview orchestrator not available')
        }
        const serverUrl = await this.previewOrchestrator.getInternalUrl(taskId)
        if (!serverUrl) {
          throw new Error('Preview server URL not available')
        }
        await this.navigateTo(taskId, '/', serverUrl)
      }

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Failed to initialize Playwright page')
      }

      taskLogger.info('Taking accessibility snapshot')

      const { page } = state
      const pageUrl = page.url()
      const pageTitle = await page.title()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      taskLogger.info(
        {
          url: pageUrl,
          title: pageTitle,
        },
        'Accessibility snapshot captured successfully',
      )

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Accessibility snapshot captured successfully\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Accessibility snapshot failed')
      throw new PlaywrightError(
        'Failed to capture accessibility snapshot',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Take screenshot of current Playwright page
   * Use this when you need to see the actual visual appearance of the UI.
   * For page structure/state only, use takeSnapshot() instead.
   * @throws PlaywrightError if screenshot fails or page not initialized
   */
  async takeScreenshot(taskId: string): Promise<ScreenshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Verify preview server is running
      await this.verifyServerRunning(taskId)

      // Auto-initialize page if it doesn't exist
      const currentUrl = this.getCurrentUrl(taskId)
      if (!currentUrl) {
        taskLogger.info('No existing Playwright page, auto-initializing to /')
        if (!this.previewOrchestrator) {
          throw new Error('Preview orchestrator not available')
        }
        const serverUrl = await this.previewOrchestrator.getInternalUrl(taskId)
        if (!serverUrl) {
          throw new Error('Preview server URL not available')
        }
        await this.navigateTo(taskId, '/', serverUrl)
      }

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Failed to initialize Playwright page')
      }

      taskLogger.info('Taking page screenshot')

      const { page } = state
      const pageUrl = page.url()
      const pageTitle = await page.title()

      // Capture screenshot
      const screenshotResult = await this.captureScreenshot(taskId, page, taskLogger)
      const screenshotBase64 = screenshotResult.screenshotBase64
      const screenshotPath = screenshotResult.screenshotPath

      if (!screenshotBase64) {
        throw new Error('Screenshot capture failed - no base64 data returned')
      }

      taskLogger.info(
        {
          url: pageUrl,
          title: pageTitle,
        },
        'Screenshot captured successfully',
      )

      // Build result with optional screenshotPath
      const result: ScreenshotResult = {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        screenshotBase64,
      }

      if (screenshotPath !== undefined) {
        result.screenshotPath = screenshotPath
      }

      return result
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Screenshot failed')
      throw new PlaywrightError(
        'Failed to capture screenshot',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Ensure preview server is ready for capture by checking status and restarting if needed
   * Handles auto-start, dirty context detection, and conditional restart logic
   * @private
   */
  private async ensureServerReadyForCapture(taskId: string): Promise<void> {
    const taskLogger = this.serviceLogger.child({ taskId })

    // Verify services are configured
    if (!this.repositoryService) {
      throw new Error('RepositoryService must be configured before capture operations')
    }

    if (!this.previewOrchestrator) {
      throw new Error('PreviewOrchestrator must be configured before capture operations')
    }

    // Check server status and auto-start if needed
    const serverStatus = await this.previewOrchestrator.getStatus(taskId)

    if (serverStatus.status !== 'started') {
      taskLogger.info(
        {
          currentStatus: serverStatus.status,
        },
        'Preview server not running, starting it',
      )
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

    taskLogger.debug(
      {
        repositoryId: task.repositoryId,
        autoReload,
      },
      'Repository config fetched',
    )

    // Check if server has dirty context (refresh status after potential start)
    const currentStatus = await this.previewOrchestrator.getStatus(taskId)
    const isDirty = currentStatus.dirtyContext ?? false

    if (isDirty && !autoReload) {
      // Code changes detected and no auto-reload - full restart needed
      taskLogger.info(
        {
          taskId,
        },
        'Restarting preview server before capture (code changes detected)',
      )

      await this.previewOrchestrator.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      await this.previewOrchestrator.startAndWaitForReady(taskId)

      taskLogger.info({ taskId }, 'Preview server restarted, ready for capture')
    } else if (isDirty && autoReload) {
      // Code changes but auto-reload enabled - skip restart
      taskLogger.info(
        {
          taskId,
        },
        'Taking capture with dirty context (auto-reload enabled)',
      )
    }
  }

  /**
   * Take accessibility snapshot with auto-restart on dirty context
   * Use this to understand page state/structure with automatic server management.
   * Checks if workspace code has changed and restarts server if needed (when autoReload is disabled)
   * @throws PlaywrightError if snapshot fails, page not initialized, or services not configured
   */
  async takeSnapshotWithAutoRestart(taskId: string): Promise<SnapshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      await this.ensureServerReadyForCapture(taskId)
      return await this.takeSnapshot(taskId)
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Snapshot with auto-restart failed')
      throw new PlaywrightError(
        'Failed to capture accessibility snapshot with auto-restart',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Take screenshot with auto-restart on dirty context
   * Use this when you need to see the actual visual appearance with automatic server management.
   * Checks if workspace code has changed and restarts server if needed (when autoReload is disabled)
   * @throws PlaywrightError if screenshot fails, page not initialized, or services not configured
   */
  async takeScreenshotWithAutoRestart(taskId: string): Promise<ScreenshotResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      await this.ensureServerReadyForCapture(taskId)
      return await this.takeScreenshot(taskId)
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Screenshot with auto-restart failed')
      throw new PlaywrightError(
        'Failed to capture screenshot with auto-restart',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Type text into an input field
   * @param taskId - Task ID
   * @param selector - CSS selector of the input element
   * @param text - Text to type
   * @param options - Optional configuration (delay between keystrokes, clear before typing)
   * @throws PlaywrightError if typing fails or page not initialized
   */
  async typeText(
    taskId: string,
    selector: string,
    text: string,
    options: { delay?: number; clear?: boolean } = {},
  ): Promise<NavigationResult> {
    const taskLogger = this.serviceLogger.child({ taskId, selector })

    try {
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info({ selector, textLength: text.length }, 'Typing text into element')

      await this.setMode(taskId, 'agent')

      const { page } = state

      // Clear existing text if requested
      if (options.clear) {
        await page.fill(selector, '')
      }

      // Type the text - only pass delay if defined
      const typeOptions: { delay?: number } = {}
      if (options.delay !== undefined) {
        typeOptions.delay = options.delay
      }
      await page.type(selector, text, typeOptions)

      // Wait for any updates
      await page.waitForTimeout(500)

      // Update current URL
      state.currentUrl = page.url()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'type', page.url())

      taskLogger.info(
        {
          url: page.url(),
          title: await page.title(),
        },
        'Text typed successfully',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Successfully typed text into element: ${selector}\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Type text failed')
      throw new PlaywrightError(
        `Failed to type text into element: ${selector}`,
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Hover over an element
   * @param taskId - Task ID
   * @param selector - CSS selector of the element to hover
   * @throws PlaywrightError if hover fails or page not initialized
   */
  async hoverElement(taskId: string, selector: string): Promise<NavigationResult> {
    const taskLogger = this.serviceLogger.child({ taskId, selector })

    try {
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info({ selector }, 'Hovering over element')

      await this.setMode(taskId, 'agent')

      const { page } = state

      // Hover over element
      await page.hover(selector)

      // Wait for hover effects
      await page.waitForTimeout(500)

      // Update current URL
      state.currentUrl = page.url()

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      // Notify lifecycle sink
      await this.notifyPlaywrightAction(taskId, 'hover', page.url())

      taskLogger.info(
        {
          url: page.url(),
          title: await page.title(),
        },
        'Hover completed successfully',
      )

      const pageUrl = page.url()
      const pageTitle = await page.title()

      return {
        url: this.getRelativePath(pageUrl),
        title: pageTitle,
        snapshot,
        message: `Successfully hovered over element: ${selector}\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Hover failed')
      throw new PlaywrightError(
        `Failed to hover over element: ${selector}`,
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Get console logs captured since last retrieval
   * @param taskId - Task ID
   * @returns Array of console messages
   * @throws PlaywrightError if retrieval fails or page not initialized
   */
  async getConsoleLogs(taskId: string): Promise<ConsoleLogsResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Auto-initialize page if it doesn't exist
      const currentUrl = this.getCurrentUrl(taskId)
      if (!currentUrl) {
        taskLogger.info('No existing Playwright page, auto-initializing to /')
        if (!this.previewOrchestrator) {
          throw new Error('Preview orchestrator not available')
        }
        const serverUrl = await this.previewOrchestrator.getInternalUrl(taskId)
        if (!serverUrl) {
          throw new Error('Preview server URL not available')
        }
        await this.verifyServerRunning(taskId)
        await this.navigateTo(taskId, '/', serverUrl)
      }

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Failed to initialize Playwright page')
      }

      taskLogger.info(
        {
          count: state.consoleMessages.length,
        },
        'Retrieving console logs',
      )

      // Get all messages and clear the array
      const messages = [...state.consoleMessages]
      state.consoleMessages.length = 0

      const formattedMessages = messages
        .map((msg) => `[${msg.type.toUpperCase()}] ${msg.text}`)
        .join('\n')

      return {
        messages,
        count: messages.length,
        message: `Retrieved ${messages.length} console messages:\n\n${formattedMessages || 'No messages'}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to retrieve console logs')
      throw new PlaywrightError(
        'Failed to retrieve console logs',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Get network requests captured since last retrieval
   * @param taskId - Task ID
   * @param urlPattern - Optional regex pattern to filter URLs
   * @returns Array of network requests
   * @throws PlaywrightError if retrieval fails or page not initialized
   */
  async getNetworkRequests(taskId: string, urlPattern?: string): Promise<NetworkRequestsResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      // Auto-initialize page if it doesn't exist
      const currentUrl = this.getCurrentUrl(taskId)
      if (!currentUrl) {
        taskLogger.info('No existing Playwright page, auto-initializing to /')
        if (!this.previewOrchestrator) {
          throw new Error('Preview orchestrator not available')
        }
        const serverUrl = await this.previewOrchestrator.getInternalUrl(taskId)
        if (!serverUrl) {
          throw new Error('Preview server URL not available')
        }
        await this.verifyServerRunning(taskId)
        await this.navigateTo(taskId, '/', serverUrl)
      }

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Failed to initialize Playwright page')
      }

      taskLogger.info(
        {
          count: state.networkRequests.length,
          filter: urlPattern,
        },
        'Retrieving network requests',
      )

      // Get all requests and clear the array
      let requests = [...state.networkRequests]
      state.networkRequests.length = 0

      // Filter by URL pattern if provided
      if (urlPattern) {
        const regex = new RegExp(urlPattern)
        requests = requests.filter((req) => regex.test(req.url))
      }

      const formattedRequests = requests
        .map((req) => `${req.method} ${req.url} - Status: ${req.status ?? 'N/A'}`)
        .join('\n')

      return {
        requests,
        count: requests.length,
        message: `Retrieved ${requests.length} network requests:\n\n${formattedRequests || 'No requests'}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Failed to retrieve network requests')
      throw new PlaywrightError(
        'Failed to retrieve network requests',
        error instanceof Error ? error : undefined,
      )
    }
  }

  /**
   * Wait for an element to appear or reach a specific state
   * @param taskId - Task ID
   * @param selector - CSS selector to wait for
   * @param options - Optional configuration (state, timeout)
   * @throws PlaywrightError if wait fails or page not initialized
   */
  async waitForElement(
    taskId: string,
    selector: string,
    options: { state?: 'attached' | 'detached' | 'visible' | 'hidden'; timeout?: number } = {},
  ): Promise<WaitResult> {
    const taskLogger = this.serviceLogger.child({ taskId, selector })

    try {
      await this.verifyServerRunning(taskId)

      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      const waitState = options.state ?? 'visible'
      const timeout = options.timeout ?? 30000

      taskLogger.info({ selector, state: waitState, timeout }, 'Waiting for element')

      const { page } = state

      // Wait for the element
      await page.waitForSelector(selector, { state: waitState, timeout })

      // Capture accessibility snapshot
      const snapshot = await this.captureAccessibilitySnapshot(page, taskLogger)

      taskLogger.info({ selector }, 'Element wait completed successfully')

      return {
        success: true,
        snapshot,
        message: `Element ${selector} reached state: ${waitState}\n\n${snapshot}`,
      }
    } catch (error) {
      taskLogger.error({ err: error as Error }, 'Wait for element failed')
      throw new PlaywrightError(
        `Failed to wait for element: ${selector}`,
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

    this.serviceLogger.debug(
      {
        taskId,
        previousMode,
        newMode: mode,
      },
      'Playwright mode changed',
    )

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
    this.serviceLogger.info(
      {
        activePages: this.pageStates.size,
      },
      'Cleaning up all Playwright resources',
    )

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
      this.serviceLogger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Error closing Playwright page',
      )
    }

    try {
      await state.browser.close()
    } catch (error) {
      this.serviceLogger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Error closing Playwright browser',
      )
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
   * Extract relative path from full URL (pathname + search + hash)
   * @param url - Full URL to extract path from
   * @returns Relative path including query parameters and hash fragment
   */
  private getRelativePath(url: string): string {
    try {
      const urlObj = new URL(url)
      return `${urlObj.pathname}${urlObj.search}${urlObj.hash}`
    } catch {
      // If URL parsing fails, return original string as fallback
      return url
    }
  }

  /**
   * Capture screenshot from Playwright page
   */
  private async captureScreenshot(
    taskId: string,
    page: Page,
    taskLogger: Logger,
  ): Promise<{ screenshotPath: string; screenshotBase64: string }> {
    // Prepare screenshot path
    const screenshotDir = path.join(getDataDir(), 'screenshots', taskId)
    await fs.mkdir(screenshotDir, { recursive: true })

    const filename = `playwright-${randomUUID()}.png`
    const screenshotPath = path.join(screenshotDir, filename)

    // Capture screenshot
    taskLogger.debug({ filename }, 'Capturing screenshot')
    await page.screenshot({
      path: screenshotPath,
      fullPage: false,
      type: 'png',
    })

    // Read screenshot and convert to base64
    const imageBuffer = await fs.readFile(screenshotPath)
    const screenshotBase64 = imageBuffer.toString('base64')

    taskLogger.debug(
      {
        path: screenshotPath,
        base64Length: screenshotBase64.length,
      },
      'Screenshot captured successfully',
    )

    return { screenshotPath, screenshotBase64 }
  }

  /**
   * Capture accessibility snapshot from Playwright page using _snapshotForAI API
   * Returns formatted snapshot matching Playwright MCP structure with element refs
   * Note: Uses private API _snapshotForAI() for AI-optimized snapshots with element references
   */
  private async captureAccessibilitySnapshot(page: Page, taskLogger: Logger): Promise<string> {
    taskLogger.debug('Capturing accessibility snapshot with _snapshotForAI API')

    try {
      const url = page.url()
      const title = await page.title()

      // Use Playwright's private _snapshotForAI API for AI-optimized snapshots
      // This provides element refs (e.g., ref=e23) for precise targeting
      // biome-ignore lint/suspicious/noExplicitAny: accessing private API for Playwright MCP compatibility
      const ariaSnapshot = await (page as any)._snapshotForAI({
        mode: 'full',
        track: 'response',
      })

      // Format response matching Playwright MCP structure
      const relativePath = this.getRelativePath(url)
      const lines: string[] = []
      lines.push('### Page state')
      lines.push(`- Page Path: ${relativePath}`)
      lines.push(`- Page Title: ${title}`)
      lines.push('- Page Snapshot:')
      lines.push('```yaml')
      lines.push(ariaSnapshot || 'No accessibility tree available')
      lines.push('```')

      const formatted = lines.join('\n')

      taskLogger.debug(
        {
          length: formatted.length,
          hasSnapshot: !!ariaSnapshot,
        },
        'Accessibility snapshot captured successfully',
      )

      return formatted
    } catch (error) {
      taskLogger.warn(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to capture accessibility snapshot',
      )
      return '### Page state\n- Error: Failed to capture accessibility snapshot'
    }
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
      this.serviceLogger.warn(
        {
          taskId,
          actionType,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to notify Playwright action',
      )
    }
  }
}
