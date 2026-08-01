/**
 * Webhook Callback Server for E2E tests.
 *
 * Spins up a lightweight HTTP server that captures outbound webhook
 * callback POSTs from the backend's WebhookProvider.send_reply().
 *
 * Modeled after SSEEventCollector — reusable with start/stop/wait semantics.
 *
 * Usage:
 *   const server = new WebhookCallbackServer();
 *   await server.start();
 *   // ... create webhook subscription loop with callback_url = server.url
 *   // ... start agent execution
 *   const callback = await server.waitForCallback(30_000);
 *   expect(callback.body).toMatchObject({ task_id: expect.any(String) });
 *   await server.stop();
 */

import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'

// =============================================================================
// Types
// =============================================================================

export interface CapturedCallback {
  /** When the callback was received */
  timestamp: number
  /** HTTP method (should always be POST) */
  method: string
  /** Request path */
  path: string
  /** Request headers (lowercased keys) */
  headers: Record<string, string>
  /** Parsed JSON body (or raw string if not JSON) */
  body: unknown
}

interface CallbackWaiter {
  resolve: (cb: CapturedCallback) => void
  reject: (err: Error) => void
  timer: ReturnType<typeof setTimeout>
}

// =============================================================================
// WebhookCallbackServer
// =============================================================================

export class WebhookCallbackServer {
  private server: Server | null = null
  private callbacks: CapturedCallback[] = []
  private waiters: CallbackWaiter[] = []
  private debug: boolean

  /** Server port (assigned after start) */
  public port = 0

  /** Full callback URL to pass to webhook subscription adapterConfig.callback_url */
  public url = ''

  constructor(options?: { debug?: boolean }) {
    this.debug = options?.debug ?? false
  }

  /**
   * Start the callback server on an OS-assigned port.
   */
  async start(): Promise<void> {
    if (this.server) {
      await this.stop()
    }

    this.callbacks = []
    this.waiters = []

    this.server = createServer((req, res) => this.handleRequest(req, res))

    await new Promise<void>((resolve, reject) => {
      this.server!.on('error', reject)
      this.server!.listen(0, '0.0.0.0', () => {
        const addr = this.server!.address()
        if (addr && typeof addr === 'object') {
          this.port = addr.port
        }
        // Use host.docker.internal when backend runs in Docker,
        // localhost when backend runs on the host.
        const host = process.env.CALLBACK_HOST || 'host.docker.internal'
        this.url = `http://${host}:${this.port}/webhook-callback`

        if (this.debug) {
          console.log(`  [CallbackServer] Listening on port ${this.port}, URL: ${this.url}`)
        }

        resolve()
      })
    })
  }

  /**
   * Wait for the next callback to arrive.
   *
   * If a callback is already queued (received before this call),
   * it resolves immediately. Otherwise waits until one arrives or timeout.
   */
  async waitForCallback(timeout = 30_000): Promise<CapturedCallback> {
    // If one arrived before this call, hand back the latest immediately.
    const arrived = this.callbacks.at(-1)
    if (arrived) {
      return arrived
    }

    return new Promise<CapturedCallback>((resolve, reject) => {
      const timer = setTimeout(() => {
        const idx = this.waiters.findIndex((w) => w.resolve === resolve)
        if (idx !== -1) {
          this.waiters.splice(idx, 1)
        }
        reject(
          new Error(
            `Timed out waiting for webhook callback after ${timeout}ms. ` +
              `Received ${this.callbacks.length} callback(s) total.`,
          ),
        )
      }, timeout)

      this.waiters.push({ resolve, reject, timer })
    })
  }

  /**
   * Wait until at least N callbacks have been received.
   */
  async waitForCallbacks(count: number, timeout = 30_000): Promise<CapturedCallback[]> {
    const startTime = Date.now()
    const pollInterval = 250

    while (Date.now() - startTime < timeout) {
      if (this.callbacks.length >= count) {
        return [...this.callbacks]
      }
      await new Promise((r) => setTimeout(r, pollInterval))
    }

    throw new Error(
      `Timed out waiting for ${count} callback(s) after ${timeout}ms. ` +
        `Received ${this.callbacks.length} callback(s).`,
    )
  }

  /**
   * Get all captured callbacks so far.
   */
  getCallbacks(): CapturedCallback[] {
    return [...this.callbacks]
  }

  /**
   * Clear all captured callbacks.
   */
  clear(): void {
    this.callbacks = []
  }

  /**
   * Stop the server and clean up.
   */
  async stop(): Promise<void> {
    // Clean up pending waiters
    for (const waiter of this.waiters) {
      clearTimeout(waiter.timer)
    }
    this.waiters = []

    if (this.server) {
      await new Promise<void>((resolve) => {
        this.server!.close(() => resolve())
      })
      this.server = null
    }

    if (this.debug) {
      console.log(`  [CallbackServer] Stopped. Captured ${this.callbacks.length} callback(s).`)
    }
  }

  // =========================================================================
  // Private Methods
  // =========================================================================

  private handleRequest(req: IncomingMessage, res: ServerResponse): void {
    const chunks: Buffer[] = []

    req.on('data', (chunk: Buffer) => {
      chunks.push(chunk)
    })

    req.on('end', () => {
      const rawBody = Buffer.concat(chunks).toString('utf-8')

      let body: unknown
      try {
        body = JSON.parse(rawBody)
      } catch {
        body = rawBody
      }

      const headers: Record<string, string> = {}
      for (const [key, value] of Object.entries(req.headers)) {
        if (typeof value === 'string') {
          headers[key] = value
        } else if (Array.isArray(value)) {
          headers[key] = value.join(', ')
        }
      }

      const callback: CapturedCallback = {
        timestamp: Date.now(),
        method: req.method || 'UNKNOWN',
        path: req.url || '/',
        headers,
        body,
      }

      if (this.debug) {
        console.log(
          `  [CallbackServer] ${callback.method} ${callback.path} — ` +
            `body: ${JSON.stringify(callback.body).slice(0, 200)}`,
        )
      }

      this.callbacks.push(callback)

      // Resolve any pending waiters
      while (this.waiters.length > 0) {
        const waiter = this.waiters.shift()!
        clearTimeout(waiter.timer)
        waiter.resolve(callback)
      }

      // Always respond 200 OK
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({ ok: true }))
    })

    req.on('error', (err) => {
      if (this.debug) {
        console.error(`  [CallbackServer] Request error: ${err.message}`)
      }
      res.writeHead(500)
      res.end()
    })
  }
}
