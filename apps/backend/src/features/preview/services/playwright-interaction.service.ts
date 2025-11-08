import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { type Browser, chromium, type Page } from '@playwright/test'
import { injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import type { PreviewServerLifecycleSink } from '../types/index.js'

export type PlaywrightMode = 'idle' | 'user' | 'agent'

interface PlaywrightPageState {
  page: Page
  browser: Browser
  mode: PlaywrightMode
  currentUrl: string
  createdAt: Date
}

interface PlaywrightActionResult {
  success: boolean
  url: string
  title: string
  snapshot?: string
  screenshotPath?: string
  screenshotBase64?: string
  error?: string
  consoleMessages?: string[]
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

  /**
   * Set lifecycle sink for emitting events
   * Called by consumers that want to receive Playwright events
   */
  setLifecycleSink(sink: PreviewServerLifecycleSink): void {
    this.lifecycleSink = sink
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
    taskLogger.info('Creating new Playwright browser and page', { initialUrl })

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
      taskLogger.warn('Initial navigation failed, page created anyway', {
        error: error instanceof Error ? error.message : String(error),
      })
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
   * Navigate Playwright page to URL
   */
  async navigateTo(
    taskId: string,
    url: string,
    serverUrl: string,
  ): Promise<PlaywrightActionResult> {
    const taskLogger = this.serviceLogger.child({ taskId, url })

    try {
      // Construct full URL
      const fullUrl = this.constructFullUrl(url, serverUrl)
      taskLogger.info('Navigating Playwright page', { fullUrl })

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

      return {
        success: true,
        url: page.url(),
        title: await page.title(),
        snapshot,
      }
    } catch (error) {
      taskLogger.error('Navigation failed', error as Error)
      return {
        success: false,
        url: '',
        title: '',
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Click element on Playwright page
   */
  async clickElement(taskId: string, selector: string): Promise<PlaywrightActionResult> {
    const taskLogger = this.serviceLogger.child({ taskId, selector })

    try {
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

      return {
        success: true,
        url: page.url(),
        title: await page.title(),
        snapshot,
      }
    } catch (error) {
      taskLogger.error('Click failed', error as Error)
      return {
        success: false,
        url: '',
        title: '',
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Evaluate JavaScript in Playwright page context
   */
  async evaluateJavaScript(
    taskId: string,
    script: string,
  ): Promise<PlaywrightActionResult & { result?: unknown }> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
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

      return {
        success: true,
        url: page.url(),
        title: await page.title(),
        result,
        snapshot,
      }
    } catch (error) {
      taskLogger.error('JavaScript evaluation failed', error as Error)
      return {
        success: false,
        url: '',
        title: '',
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Fill form fields on Playwright page
   */
  async fillForm(taskId: string, fields: Record<string, string>): Promise<PlaywrightActionResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
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

      return {
        success: true,
        url: page.url(),
        title: await page.title(),
        snapshot,
      }
    } catch (error) {
      taskLogger.error('Form fill failed', error as Error)
      return {
        success: false,
        url: '',
        title: '',
        error: error instanceof Error ? error.message : String(error),
      }
    }
  }

  /**
   * Take screenshot of current Playwright page state
   */
  async takeSnapshot(taskId: string, includeScreenshot = true): Promise<PlaywrightActionResult> {
    const taskLogger = this.serviceLogger.child({ taskId })

    try {
      const state = this.pageStates.get(taskId)
      if (!state) {
        throw new Error('Playwright page not initialized. Call navigateTo first.')
      }

      taskLogger.info('Taking page snapshot', { includeScreenshot })

      const { page } = state

      let screenshotResult = {}
      if (includeScreenshot) {
        screenshotResult = await this.captureScreenshot(taskId, page, taskLogger)
      }

      taskLogger.info('Snapshot captured successfully', {
        url: page.url(),
        title: await page.title(),
      })

      return {
        success: true,
        url: page.url(),
        title: await page.title(),
        ...screenshotResult,
      }
    } catch (error) {
      taskLogger.error('Snapshot failed', error as Error)
      return {
        success: false,
        url: '',
        title: '',
        error: error instanceof Error ? error.message : String(error),
      }
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
   * Construct full URL from path and server URL
   */
  private constructFullUrl(urlOrPath: string, serverUrl: string): string {
    // If already a full URL, return as-is
    if (urlOrPath.startsWith('http://') || urlOrPath.startsWith('https://')) {
      return urlOrPath
    }

    // Otherwise, construct from server URL and path
    const baseUrl = serverUrl.endsWith('/') ? serverUrl.slice(0, -1) : serverUrl
    const path = urlOrPath.startsWith('/') ? urlOrPath : `/${urlOrPath}`
    return `${baseUrl}${path}`
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
