/**
 * Standalone Browser Client for Sandbox Browser Automation
 *
 * This file is self-contained and can be imported directly from scripts
 * running inside the sandbox container without any @moatless/* dependencies.
 *
 * All page management happens locally via CDP - the server only handles:
 * - Browser lifecycle (start/stop)
 * - WebSocket endpoint for CDP connections
 * - Serving screenshot files from disk
 *
 * Usage:
 * ```typescript
 * import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";
 *
 * // Auto-discovers sandbox server from PORT env (default: 8001)
 * const client = await connect();
 * const page = await client.page("main");
 *
 * // Use getPreviewUrl() to navigate to the preview server
 * await page.goto(getPreviewUrl());           // http://localhost:3000
 * await page.goto(getPreviewUrl("/about"));   // http://localhost:3000/about
 * await waitForPageLoad(page);
 *
 * const snapshot = await client.getAISnapshot("main");
 * console.log(snapshot);
 *
 * await client.disconnect();
 * ```
 */

import { mkdir, writeFile } from 'node:fs/promises'

// ============================================================================
// CDP Protocol Types and Client (using Bun's native WebSocket)
// ============================================================================

/**
 * CDP JSON-RPC request message
 */
interface CDPRequest {
  id: number
  method: string
  params?: Record<string, unknown>
  sessionId?: string
}

/**
 * CDP JSON-RPC response message
 */
interface CDPResponse {
  id?: number
  result?: Record<string, unknown>
  error?: { code: number; message: string; data?: string }
  method?: string
  params?: Record<string, unknown>
  sessionId?: string
}

/**
 * Low-level CDP WebSocket client using Bun's native WebSocket
 */
class CDPClient {
  private ws: WebSocket | undefined
  private requestId = 0
  private pendingRequests = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (reason: Error) => void }
  >()
  private eventListeners = new Map<string, Set<(params: unknown) => void>>()
  private sessionEventListeners = new Map<string, Map<string, Set<(params: unknown) => void>>>()
  private connected = false

  /**
   * Connect to Chrome DevTools Protocol via WebSocket
   * @param httpEndpoint - The browser's HTTP endpoint (e.g., http://localhost:9222)
   */
  async connect(httpEndpoint: string): Promise<void> {
    // Fetch WebSocket URL from browser's JSON endpoint
    const versionUrl = `${httpEndpoint}/json/version`
    const response = await fetch(versionUrl)
    if (!response.ok) {
      throw new Error(`Failed to get browser version info: ${response.statusText}`)
    }
    const versionInfo = (await response.json()) as { webSocketDebuggerUrl: string }
    const wsUrl = versionInfo.webSocketDebuggerUrl

    // Connect using Bun's native WebSocket
    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(wsUrl)

      this.ws.onopen = () => {
        this.connected = true
        resolve()
      }

      this.ws.onerror = (event) => {
        reject(new Error(`WebSocket connection error: ${event}`))
      }

      this.ws.onclose = () => {
        this.connected = false
        // Reject all pending requests
        for (const [, { reject: rejectPending }] of this.pendingRequests) {
          rejectPending(new Error('WebSocket connection closed'))
        }
        this.pendingRequests.clear()
      }

      this.ws.onmessage = (event) => {
        this.handleMessage(event.data as string)
      }
    })
  }

  /**
   * Handle incoming CDP messages
   */
  private handleMessage(data: string): void {
    const message: CDPResponse = JSON.parse(data)

    // Handle response to a request
    if (message.id !== undefined) {
      this.handleResponse(message)
      return
    }

    // Handle event
    if (message.method) {
      this.handleEvent(message)
    }
  }

  /**
   * Handle CDP response messages
   */
  private handleResponse(message: CDPResponse): void {
    const pending = this.pendingRequests.get(message.id!)
    if (!pending) {
      return
    }
    this.pendingRequests.delete(message.id!)
    if (message.error) {
      pending.reject(new Error(`CDP Error: ${message.error.message} (${message.error.code})`))
    } else {
      pending.resolve(message.result)
    }
  }

  /**
   * Handle CDP event messages
   */
  private handleEvent(message: CDPResponse): void {
    // Session-specific event
    if (message.sessionId) {
      this.emitSessionEvent(message.sessionId, message.method!, message.params)
    }
    // Global event
    this.emitGlobalEvent(message.method!, message.params)
  }

  /**
   * Emit event to session-specific listeners
   */
  private emitSessionEvent(sessionId: string, method: string, params: unknown): void {
    const sessionListeners = this.sessionEventListeners.get(sessionId)
    if (!sessionListeners) {
      return
    }
    const listeners = sessionListeners.get(method)
    if (!listeners) {
      return
    }
    for (const listener of listeners) {
      listener(params)
    }
  }

  /**
   * Emit event to global listeners
   */
  private emitGlobalEvent(method: string, params: unknown): void {
    const listeners = this.eventListeners.get(method)
    if (!listeners) {
      return
    }
    for (const listener of listeners) {
      listener(params)
    }
  }

  /**
   * Send a CDP command and wait for response
   */
  send<T = Record<string, unknown>>(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<T> {
    if (!(this.ws && this.connected)) {
      return Promise.reject(new Error('CDP client not connected'))
    }

    const id = ++this.requestId
    const request: CDPRequest = { id, method }
    if (params) {
      request.params = params
    }
    if (sessionId) {
      request.sessionId = sessionId
    }

    return new Promise((resolve, reject) => {
      this.pendingRequests.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      })
      this.ws?.send(JSON.stringify(request))
    })
  }

  /**
   * Subscribe to a CDP event
   */
  on(event: string, callback: (params: unknown) => void, sessionId?: string): void {
    if (sessionId) {
      let sessionListeners = this.sessionEventListeners.get(sessionId)
      if (!sessionListeners) {
        sessionListeners = new Map()
        this.sessionEventListeners.set(sessionId, sessionListeners)
      }
      let listeners = sessionListeners.get(event)
      if (!listeners) {
        listeners = new Set()
        sessionListeners.set(event, listeners)
      }
      listeners.add(callback)
    } else {
      let listeners = this.eventListeners.get(event)
      if (!listeners) {
        listeners = new Set()
        this.eventListeners.set(event, listeners)
      }
      listeners.add(callback)
    }
  }

  /**
   * Remove event listener
   */
  off(event: string, callback: (params: unknown) => void, sessionId?: string): void {
    if (sessionId) {
      const sessionListeners = this.sessionEventListeners.get(sessionId)
      if (sessionListeners) {
        const listeners = sessionListeners.get(event)
        if (listeners) {
          listeners.delete(callback)
        }
      }
    } else {
      const listeners = this.eventListeners.get(event)
      if (listeners) {
        listeners.delete(callback)
      }
    }
  }

  /**
   * Check if connected
   */
  isConnected(): boolean {
    return this.connected && this.ws?.readyState === WebSocket.OPEN
  }

  /**
   * Disconnect from CDP
   */
  disconnect(): void {
    if (this.ws) {
      this.ws.close()
      this.ws = undefined
    }
    this.connected = false
    this.pendingRequests.clear()
    this.eventListeners.clear()
    this.sessionEventListeners.clear()
  }
}

// ============================================================================
// CDPPage - Playwright Page-like interface backed by CDP
// ============================================================================

/**
 * A Playwright Page-like class that uses CDP directly
 */
class CDPPage {
  private cdpClient: CDPClient
  private sessionId: string
  private targetId: string
  private currentUrl = 'about:blank'
  private viewport = { width: 1280, height: 720 }
  private closed = false
  private closeCallbacks: Set<() => void> = new Set()

  constructor(cdpClient: CDPClient, sessionId: string, targetId: string) {
    this.cdpClient = cdpClient
    this.sessionId = sessionId
    this.targetId = targetId

    // Listen for target destroyed event
    this.cdpClient.on('Target.targetDestroyed', (params: unknown) => {
      const p = params as { targetId: string }
      if (p.targetId === this.targetId) {
        this.closed = true
        for (const callback of this.closeCallbacks) {
          callback()
        }
      }
    })
  }

  /**
   * Initialize the page (enable required domains)
   */
  async initialize(): Promise<void> {
    await Promise.all([
      this.cdpClient.send('Page.enable', {}, this.sessionId),
      this.cdpClient.send('DOM.enable', {}, this.sessionId),
      this.cdpClient.send('Runtime.enable', {}, this.sessionId),
      this.cdpClient.send('Accessibility.enable', {}, this.sessionId),
    ])
  }

  /**
   * Navigate to a URL
   */
  async goto(
    url: string,
    options?: { waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'; timeout?: number },
  ): Promise<void> {
    const waitUntil = options?.waitUntil ?? 'domcontentloaded'
    const timeout = options?.timeout ?? 30000

    // Set up event listener for navigation complete
    const navigationPromise = new Promise<void>((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        reject(new Error(`Navigation timeout after ${timeout}ms`))
      }, timeout)

      const eventName = waitUntil === 'load' ? 'Page.loadEventFired' : 'Page.domContentEventFired'

      const handler = () => {
        clearTimeout(timeoutId)
        this.cdpClient.off(eventName, handler, this.sessionId)
        resolve()
      }
      this.cdpClient.on(eventName, handler, this.sessionId)
    })

    // Navigate
    const result = await this.cdpClient.send<{ frameId: string; errorText?: string }>(
      'Page.navigate',
      { url },
      this.sessionId,
    )

    if (result.errorText) {
      throw new Error(`Navigation failed: ${result.errorText}`)
    }

    this.currentUrl = url

    // Wait for navigation to complete
    await navigationPromise
  }

  /**
   * Capture a screenshot
   */
  async screenshot(options?: {
    path?: string
    type?: 'png' | 'jpeg'
    fullPage?: boolean
  }): Promise<Buffer> {
    const format = options?.type ?? 'png'

    // If fullPage, we need to get the full page dimensions first
    let clip: { x: number; y: number; width: number; height: number; scale: number } | undefined
    if (options?.fullPage) {
      // Get layout metrics for full page
      const metrics = await this.cdpClient.send<{
        contentSize: { width: number; height: number }
      }>('Page.getLayoutMetrics', {}, this.sessionId)
      clip = {
        x: 0,
        y: 0,
        width: metrics.contentSize.width,
        height: metrics.contentSize.height,
        scale: 1,
      }
    }

    const result = await this.cdpClient.send<{ data: string }>(
      'Page.captureScreenshot',
      {
        format,
        ...(clip ? { clip } : {}),
      },
      this.sessionId,
    )

    const buffer = Buffer.from(result.data, 'base64')

    // Save to file if path specified
    if (options?.path) {
      await writeFile(options.path, buffer)
    }

    return buffer
  }

  /**
   * Click on an element
   */
  async click(selector: string): Promise<void> {
    // Get document root
    const doc = await this.cdpClient.send<{ root: { nodeId: number } }>(
      'DOM.getDocument',
      {},
      this.sessionId,
    )

    // Query for the element
    const node = await this.cdpClient.send<{ nodeId: number }>(
      'DOM.querySelector',
      { nodeId: doc.root.nodeId, selector },
      this.sessionId,
    )

    if (!node.nodeId) {
      throw new Error(`Element not found: ${selector}`)
    }

    // Get box model to find clickable point
    const boxModel = await this.cdpClient.send<{
      model: { content: number[] }
    }>('DOM.getBoxModel', { nodeId: node.nodeId }, this.sessionId)

    // Calculate center point from content quad (x1,y1,x2,y2,x3,y3,x4,y4)
    const quad = boxModel.model.content
    const x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4
    const y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4

    // Dispatch mouse events
    await this.cdpClient.send(
      'Input.dispatchMouseEvent',
      { type: 'mouseMoved', x, y },
      this.sessionId,
    )
    await this.cdpClient.send(
      'Input.dispatchMouseEvent',
      { type: 'mousePressed', x, y, button: 'left', clickCount: 1 },
      this.sessionId,
    )
    await this.cdpClient.send(
      'Input.dispatchMouseEvent',
      { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 },
      this.sessionId,
    )
  }

  /**
   * Fill text into an element
   */
  async fill(selector: string, text: string): Promise<void> {
    // Focus the element first using JavaScript
    await this.cdpClient.send(
      'Runtime.evaluate',
      {
        expression: `document.querySelector(${JSON.stringify(selector)})?.focus()`,
      },
      this.sessionId,
    )

    // Clear existing content
    await this.cdpClient.send(
      'Runtime.evaluate',
      {
        expression: `document.querySelector(${JSON.stringify(selector)}).value = ''`,
      },
      this.sessionId,
    )

    // Insert the text
    await this.cdpClient.send('Input.insertText', { text }, this.sessionId)
  }

  /**
   * Wait for load state
   */
  async waitForLoadState(
    state: 'load' | 'domcontentloaded' | 'networkidle' = 'domcontentloaded',
    options?: { timeout?: number },
  ): Promise<void> {
    const timeout = options?.timeout ?? 30000

    // For networkidle, we'll just do a short wait since proper implementation requires tracking requests
    if (state === 'networkidle') {
      await this.waitForTimeout(500)
      return
    }

    const eventName = state === 'load' ? 'Page.loadEventFired' : 'Page.domContentEventFired'

    return new Promise((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        this.cdpClient.off(eventName, handler, this.sessionId)
        reject(new Error(`Timeout waiting for ${state}`))
      }, timeout)

      const handler = () => {
        clearTimeout(timeoutId)
        this.cdpClient.off(eventName, handler, this.sessionId)
        resolve()
      }
      this.cdpClient.on(eventName, handler, this.sessionId)
    })
  }

  /**
   * Wait for a specified timeout
   */
  waitForTimeout(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * Get the current URL
   */
  url(): string {
    return this.currentUrl
  }

  /**
   * Get viewport size
   */
  viewportSize(): { width: number; height: number } | null {
    return this.viewport
  }

  /**
   * Set viewport size
   */
  async setViewportSize(size: { width: number; height: number }): Promise<void> {
    this.viewport = size
    await this.cdpClient.send(
      'Emulation.setDeviceMetricsOverride',
      {
        width: size.width,
        height: size.height,
        deviceScaleFactor: 1,
        mobile: false,
      },
      this.sessionId,
    )
  }

  /**
   * Check if page is closed
   */
  isClosed(): boolean {
    return this.closed
  }

  /**
   * Close the page
   */
  async close(): Promise<void> {
    if (this.closed) {
      return
    }
    await this.cdpClient.send('Target.closeTarget', { targetId: this.targetId })
    this.closed = true
    for (const callback of this.closeCallbacks) {
      callback()
    }
  }

  /**
   * Register close event listener
   */
  on(event: 'close', callback: () => void): void {
    if (event === 'close') {
      this.closeCallbacks.add(callback)
    }
  }

  /**
   * Create a locator (for ariaSnapshot)
   */
  locator(_selector: string): { ariaSnapshot: () => Promise<string> } {
    return {
      ariaSnapshot: async (): Promise<string> => {
        // Get accessibility tree
        const result = await this.cdpClient.send<{
          nodes: Array<{
            nodeId: string
            role: { value: string }
            name?: { value: string }
            properties?: Array<{ name: string; value: { value: unknown } }>
            childIds?: string[]
          }>
        }>('Accessibility.getFullAXTree', {}, this.sessionId)

        // Convert to YAML-like format matching Playwright's output
        return this.formatAccessibilityTree(result.nodes)
      },
    }
  }

  /**
   * Format accessibility tree as YAML (matching Playwright's ariaSnapshot format)
   */
  private formatAccessibilityTree(
    nodes: Array<{
      nodeId: string
      role: { value: string }
      name?: { value: string }
      properties?: Array<{ name: string; value: { value: unknown } }>
      childIds?: string[]
    }>,
  ): string {
    type AXNode = (typeof nodes)[number]
    const nodeMap = new Map(nodes.map((n) => [n.nodeId, n]))
    const lines: string[] = []
    const SKIP_ROLES = new Set(['none', 'generic', 'RootWebArea'])

    const extractProps = (node: AXNode): string[] => {
      const props: string[] = []
      for (const prop of node.properties || []) {
        const val = prop.value.value
        if (prop.name === 'level' && val) {
          props.push(`level=${val}`)
        } else if (prop.name === 'checked' && val !== undefined) {
          props.push(`checked=${val}`)
        } else if (prop.name === 'selected' && val !== undefined) {
          props.push(`selected=${val}`)
        }
      }
      return props
    }

    const formatNodeLine = (
      role: string,
      name: string | undefined,
      props: string[],
      indent: number,
    ): string => {
      const indentStr = '  '.repeat(indent)
      let line = `${indentStr}- ${role}`
      if (name) {
        line += ` "${name}"`
      }
      if (props.length > 0) {
        line += ` [${props.join(', ')}]`
      }
      return line
    }

    const processNode = (nodeId: string, indent: number): void => {
      const node = nodeMap.get(nodeId)
      if (!node) {
        return
      }

      const role = node.role.value
      const childIds = node.childIds || []

      // Skip generic/ignored roles - process children directly
      if (SKIP_ROLES.has(role)) {
        for (const childId of childIds) {
          processNode(childId, indent)
        }
        return
      }

      const name = node.name?.value
      // Only add non-empty meaningful nodes
      if (role !== 'text' || name) {
        const props = extractProps(node)
        lines.push(formatNodeLine(role, name, props, indent))
      }

      // Process children
      for (const childId of childIds) {
        processNode(childId, indent + 1)
      }
    }

    // Find root and start formatting
    const root = nodes.find((n) => n.role.value === 'RootWebArea')
    if (root) {
      for (const childId of root.childIds || []) {
        processNode(childId, 0)
      }
    }

    return lines.join('\n')
  }

  /**
   * Get session ID for this page
   */
  getSessionId(): string {
    return this.sessionId
  }

  /**
   * Get target ID for this page
   */
  getTargetId(): string {
    return this.targetId
  }
}

// ============================================================================
// CDPBrowserContext - Manages page creation via CDP
// ============================================================================

/**
 * Browser context that manages pages via CDP
 */
class CDPBrowserContext {
  private cdpClient: CDPClient
  private pages: CDPPage[] = []
  private viewport = { width: 1280, height: 720 }

  constructor(cdpClient: CDPClient, viewport?: { width: number; height: number }) {
    this.cdpClient = cdpClient
    if (viewport) {
      this.viewport = viewport
    }
  }

  /**
   * Create a new page
   */
  async newPage(): Promise<CDPPage> {
    // Create a new target (tab)
    const result = await this.cdpClient.send<{ targetId: string }>('Target.createTarget', {
      url: 'about:blank',
    })

    // Attach to the target
    const attachResult = await this.cdpClient.send<{ sessionId: string }>('Target.attachToTarget', {
      targetId: result.targetId,
      flatten: true,
    })

    // Create CDPPage wrapper
    const page = new CDPPage(this.cdpClient, attachResult.sessionId, result.targetId)
    await page.initialize()
    await page.setViewportSize(this.viewport)

    this.pages.push(page)
    return page
  }

  /**
   * Get all pages
   */
  getPages(): CDPPage[] {
    return this.pages.filter((p) => !p.isClosed())
  }
}

// ============================================================================
// Types (embedded to avoid external dependencies)
// ============================================================================

/**
 * Browser action types that trigger screenshot capture
 */
export type BrowserAction = 'navigate' | 'click' | 'fill' | 'manual'

/**
 * Options for connecting to browser
 */
export interface ConnectOptions {
  /** Default viewport size (default: 1280x720) */
  viewport?: { width: number; height: number }
}

/**
 * Options for creating or getting a page
 */
export interface GetPageOptions {
  /** Default URL to navigate to if creating new page */
  url?: string
  /** Override viewport for this page */
  viewport?: { width: number; height: number }
  /** Disable automatic screenshot capture after actions (default: false) */
  disableAutoCapture?: boolean
}

/**
 * Options for waiting for page load
 */
export interface WaitForLoadOptions {
  /** Timeout in milliseconds (default: 30000) */
  timeout?: number
  /** Wait until condition (default: 'domcontentloaded') */
  waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'
}

/**
 * Screenshot metadata written to disk
 */
interface ScreenshotMetadata {
  filename: string
  action: BrowserAction
  pageName: string
  url: string
  timestamp: string
  width: number
  height: number
}

/**
 * Page interface matching CDPPage's public API
 * This provides a Playwright-like interface for automation scripts
 */
export interface IPage {
  goto(
    url: string,
    options?: { waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'; timeout?: number },
  ): Promise<void>
  screenshot(options?: {
    path?: string
    type?: 'png' | 'jpeg'
    fullPage?: boolean
  }): Promise<Buffer>
  click(selector: string): Promise<void>
  fill(selector: string, text: string): Promise<void>
  waitForLoadState(
    state?: 'load' | 'domcontentloaded' | 'networkidle',
    options?: { timeout?: number },
  ): Promise<void>
  waitForTimeout(ms: number): Promise<void>
  url(): string
  viewportSize(): { width: number; height: number } | null
  setViewportSize(size: { width: number; height: number }): Promise<void>
  isClosed(): boolean
  close(): Promise<void>
  on(event: 'close', callback: () => void): void
  locator(selector: string): { ariaSnapshot(): Promise<string> }
}

/**
 * Browser client interface for scripts
 */
export interface IBrowserClient {
  /** Get or create a named page - returns CDPPage with auto-capture */
  page(name: string, options?: GetPageOptions): Promise<IPage>

  /** List all named pages */
  list(): Promise<string[]>

  /** Close a named page */
  close(name: string): Promise<void>

  /** Disconnect from browser server */
  disconnect(): Promise<void>

  /** Get ARIA snapshot of a page */
  getAISnapshot(name: string): Promise<string>

  /** Manually capture a screenshot for a page */
  captureScreenshot(pageName: string, action?: BrowserAction): Promise<void>
}

// ============================================================================
// Browser Client Implementation
// ============================================================================

/**
 * Get the default sandbox server URL from environment.
 * Scripts running inside sandbox can use this to auto-discover the server.
 */
function getDefaultServerUrl(): string {
  const port = process.env['PORT'] || '8001'
  return `http://localhost:${port}`
}

/**
 * Get the preview server URL.
 * The preview server always runs on port 3000 inside the sandbox.
 * @param path - Optional path to append (e.g., "/about")
 * @returns The full URL to the preview server
 */
export function getPreviewUrl(path?: string): string {
  const baseUrl = 'http://localhost:3000'
  if (!path) {
    return baseUrl
  }
  // Ensure path starts with /
  const normalizedPath = path.startsWith('/') ? path : `/${path}`
  return `${baseUrl}${normalizedPath}`
}

/**
 * Browser client for automation scripts
 * Manages pages locally via CDP - server only provides browser lifecycle
 */
export class BrowserClient implements IBrowserClient {
  private cdpClient: CDPClient | undefined
  private context: CDPBrowserContext | undefined
  private pages: Map<string, CDPPage> = new Map()
  private pageAutoCapture: Map<string, boolean> = new Map()
  private screenshotIndex = 0
  private readonly serverUrl: string
  private readonly screenshotDir: string
  private defaultViewport = { width: 1280, height: 720 }

  constructor(serverUrl?: string) {
    // Normalize URL (remove trailing slash), auto-discover if not provided
    const url = serverUrl || getDefaultServerUrl()
    this.serverUrl = url.replace(/\/$/, '')
    this.screenshotDir = process.env['DATA_DIR']
      ? `${process.env['DATA_DIR']}/screenshots`
      : '/data/screenshots'
  }

  /**
   * Capture a screenshot by writing directly to disk
   * @param pageName - Name of the page
   * @param action - Action that triggered the screenshot (default: 'manual')
   */
  async captureScreenshot(pageName: string, action: BrowserAction = 'manual'): Promise<void> {
    const page = this.pages.get(pageName)
    if (!page || page.isClosed()) {
      return
    }

    try {
      // Ensure directory exists
      await mkdir(this.screenshotDir, { recursive: true })

      // Generate filename with index, action, and timestamp
      const index = String(this.screenshotIndex++).padStart(3, '0')
      const timestamp = Date.now()
      const filename = `${index}_${action}_${timestamp}.png`
      const filepath = `${this.screenshotDir}/${filename}`

      // Take screenshot
      await page.screenshot({ path: filepath, type: 'png' })

      // Get viewport size
      const viewport = page.viewportSize() || this.defaultViewport

      // Write metadata JSON
      const metadata: ScreenshotMetadata = {
        filename,
        action,
        pageName,
        url: page.url(),
        timestamp: new Date().toISOString(),
        width: viewport.width,
        height: viewport.height,
      }
      await writeFile(`${filepath}.json`, JSON.stringify(metadata, null, 2))
    } catch {
      // Silently ignore capture errors - they shouldn't break automation
    }
  }

  /**
   * Wrap a CDPPage object to auto-capture screenshots after key actions
   */
  private wrapPageWithAutoCapture(page: CDPPage, pageName: string): CDPPage {
    const captureAfter = async (action: BrowserAction) => {
      // Check if auto-capture is enabled for this page
      if (this.pageAutoCapture.get(pageName) === false) {
        return
      }
      await this.captureScreenshot(pageName, action)
    }

    // Wrap goto
    const originalGoto = page.goto.bind(page)
    page.goto = async (
      url: string,
      options?: { waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'; timeout?: number },
    ) => {
      await originalGoto(url, options)
      await captureAfter('navigate')
    }

    // Wrap click
    const originalClick = page.click.bind(page)
    page.click = async (selector: string) => {
      await originalClick(selector)
      await captureAfter('click')
    }

    // Wrap fill
    const originalFill = page.fill.bind(page)
    page.fill = async (selector: string, text: string) => {
      await originalFill(selector, text)
      await captureAfter('fill')
    }

    return page
  }

  /**
   * Connect to the browser server via CDP using Bun's native WebSocket
   */
  async connect(options?: ConnectOptions): Promise<void> {
    if (options?.viewport) {
      this.defaultViewport = options.viewport
    }

    // Get server info to check if browser is running
    const infoResponse = await fetch(`${this.serverUrl}/browser`)
    if (!infoResponse.ok) {
      throw new Error(`Failed to get browser server info: ${infoResponse.statusText}`)
    }

    let info = (await infoResponse.json()) as {
      running: boolean
      wsEndpoint?: string
      cdpPort?: number
    }

    if (!info.running) {
      // Start the browser server
      const startResponse = await fetch(`${this.serverUrl}/browser/start`, {
        method: 'POST',
      })
      if (!startResponse.ok) {
        const error = (await startResponse.json().catch(() => ({ error: 'Unknown error' }))) as {
          error?: string
        }
        throw new Error(
          `Failed to start browser server: ${error.error || startResponse.statusText}`,
        )
      }
      info = (await startResponse.json()) as {
        running: boolean
        wsEndpoint?: string
        cdpPort?: number
      }
    }

    // Get CDP port - prefer cdpPort field, fallback to extracting from wsEndpoint
    let cdpPort: number
    if (info.cdpPort) {
      cdpPort = info.cdpPort
    } else if (info.wsEndpoint) {
      // Extract port from wsEndpoint like "http://localhost:9223" or "ws://localhost:9222/devtools/browser/..."
      const portMatch = info.wsEndpoint.match(/:(\d+)/)
      if (portMatch) {
        cdpPort = Number.parseInt(portMatch[1], 10)
      } else {
        throw new Error('Could not extract CDP port from wsEndpoint')
      }
    } else {
      throw new Error('Browser server is running but no CDP port or WebSocket endpoint available')
    }

    // Connect to browser via native CDP client (uses Bun's WebSocket)
    this.cdpClient = new CDPClient()
    await this.cdpClient.connect(`http://localhost:${cdpPort}`)

    // Create browser context with default viewport
    this.context = new CDPBrowserContext(this.cdpClient, this.defaultViewport)
  }

  /**
   * Get or create a named page
   * Returns a CDPPage object with auto-capture enabled by default
   */
  async page(name: string, options?: GetPageOptions): Promise<CDPPage> {
    if (!(this.cdpClient && this.context)) {
      throw new Error('Not connected to browser server. Call connect() first.')
    }

    // Check if we already have this page locally
    const existingPage = this.pages.get(name)
    if (existingPage && !existingPage.isClosed()) {
      return existingPage
    }

    // Create new page locally
    let page = await this.context.newPage()

    // Set custom viewport if specified
    if (options?.viewport) {
      await page.setViewportSize(options.viewport)
    }

    // Navigate to URL if provided
    if (options?.url) {
      await page.goto(options.url, {
        waitUntil: 'domcontentloaded',
        timeout: 30000,
      })
    }

    // Track auto-capture setting for this page
    this.pageAutoCapture.set(name, !options?.disableAutoCapture)

    // Wrap page with auto-capture (unless disabled)
    if (!options?.disableAutoCapture) {
      page = this.wrapPageWithAutoCapture(page, name)
    }

    // Cache the page
    this.pages.set(name, page)

    // Remove from cache when closed
    page.on('close', () => {
      this.pages.delete(name)
      this.pageAutoCapture.delete(name)
    })

    return page
  }

  /**
   * List all named pages
   */
  list(): Promise<string[]> {
    // Return locally tracked pages (filter out closed ones)
    const activePages: string[] = []
    for (const [name, page] of this.pages) {
      if (!page.isClosed()) {
        activePages.push(name)
      } else {
        this.pages.delete(name)
        this.pageAutoCapture.delete(name)
      }
    }
    return Promise.resolve(activePages)
  }

  /**
   * Close a named page
   */
  async close(name: string): Promise<void> {
    const page = this.pages.get(name)
    if (page && !page.isClosed()) {
      await page.close()
    }
    this.pages.delete(name)
    this.pageAutoCapture.delete(name)
  }

  /**
   * Disconnect from the browser server
   * Note: We disconnect but don't close the browser - it stays alive for next script
   */
  disconnect(): Promise<void> {
    // Clear local page caches
    this.pages.clear()
    this.pageAutoCapture.clear()

    // Disconnect from browser (don't close it - browser stays alive)
    if (this.cdpClient) {
      this.cdpClient.disconnect()
      this.cdpClient = undefined
      this.context = undefined
    }
    return Promise.resolve()
  }

  /**
   * Get AI-friendly ARIA snapshot of a page
   * Done client-side using CDP's Accessibility API
   */
  async getAISnapshot(name: string): Promise<string> {
    const page = this.pages.get(name)
    if (!page || page.isClosed()) {
      throw new Error(`Page '${name}' not found. Call page() first.`)
    }

    // Use CDPPage's ariaSnapshot via the locator interface
    const snapshot = await page.locator(':root').ariaSnapshot()

    if (!snapshot) {
      throw new Error('Failed to get accessibility snapshot - page may not be loaded')
    }

    return snapshot
  }
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Connect to a browser server
 * @param options - Connection options including viewport size
 */
export async function connect(options?: ConnectOptions): Promise<BrowserClient> {
  const client = new BrowserClient()
  await client.connect(options)
  return client
}

/**
 * Wait for page to be fully loaded
 * Combines multiple wait strategies for reliable page readiness
 */
export async function waitForPageLoad(
  page: IPage,
  options: WaitForLoadOptions = {},
): Promise<void> {
  const { timeout = 30000, waitUntil = 'domcontentloaded' } = options

  // Wait for the specified load state
  await page.waitForLoadState(waitUntil, { timeout })

  // Additional stabilization - wait for network to be idle briefly
  try {
    await page.waitForLoadState('networkidle', { timeout: 5000 })
  } catch {
    // Network idle timeout is acceptable - page might have long-polling
  }

  // Small delay for any final rendering
  await page.waitForTimeout(100)
}

/**
 * Take a screenshot and return as base64
 */
export async function takeScreenshot(
  page: IPage,
  options?: { fullPage?: boolean },
): Promise<string> {
  const buffer = await page.screenshot({
    fullPage: options?.fullPage ?? false,
    type: 'png',
  })
  return buffer.toString('base64')
}
