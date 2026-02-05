import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import { Readable } from 'node:stream'
import { URL } from 'node:url'
import type { UserSession } from '@moatless/types'
import { injectable } from 'inversify'
import { LRUCache } from 'lru-cache'
import { logger } from '../../../utils/logger.js'
import { SessionContext } from '../../auth/services/session-context.js'
import { SESSION_COOKIE_NAME } from '../../auth/utils/cookie.utils.js'
import { loadProxyConfig } from '../config/proxy-config.js'
import type {
  HostnameProxyConfig,
  ResolutionFailureReason,
  TargetInfo,
  TargetResolver,
} from '../types/index.js'
import { renderBadGatewayPage, renderProxyErrorPage } from '../utils/error-pages.js'
import { getCookieValue, getProxyRequestSession } from '../utils/proxy-auth.utils.js'
import { extractSubdomain, parseSubdomain } from '../utils/proxy-core.js'

/**
 * HostnameProxyService - Thin router for hostname-based preview proxying
 *
 * Routes requests based on subdomain to corresponding sandbox pods.
 * Uses TargetResolver interface to find the target pod.
 *
 * Responsibilities:
 * - Session cookie validation (auth check)
 * - Extract task ID from hostname
 * - Find pod IP via TargetResolver
 * - Forward request to sandbox's /preview endpoint
 *
 * The sandbox handles all preview logic including HTML injection.
 *
 * Example: task-123.proxy.moatless.xyz → routes to sandbox pod's /preview endpoint
 *
 * Configured via environment variables:
 * - USE_HOSTNAME_ROUTING=true - Enable hostname routing
 * - PROXY_PORT - Port to listen on (default: 80)
 * - PROXY_DOMAIN - Base domain (e.g., 'proxy.moatless.xyz')
 * - PROXY_PROTOCOL - http or https (default: http)
 */
/** Request timeout for proxy connections in milliseconds */
const PROXY_REQUEST_TIMEOUT_MS = 30_000

/** Timeout for preparation phase (auth + resolution) in milliseconds */
const PREP_PHASE_TIMEOUT_MS = 10_000

/**
 * Wraps a promise with a timeout.
 * Rejects with a TimeoutError if the promise doesn't resolve within the specified time.
 */
class TimeoutError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'TimeoutError'
  }
}

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new TimeoutError(message)), ms)
    promise
      .then((value) => {
        clearTimeout(timer)
        resolve(value)
      })
      .catch((err) => {
        clearTimeout(timer)
        reject(err)
      })
  })
}

@injectable()
export class HostnameProxyService {
  private server: http.Server | null = null
  private readonly serviceLogger: typeof logger
  private static instance: HostnameProxyService | null = null
  private readonly serverHost: string
  private readonly sessionCache = new LRUCache<string, UserSession>({
    max: 1000,
    ttl: 60_000, // 1 minute
  })

  constructor(
    private readonly config: HostnameProxyConfig,
    private readonly targetResolver: TargetResolver,
    serverHost?: string,
  ) {
    this.serviceLogger = logger.child({ service: 'hostname-proxy' })
    this.serverHost = serverHost || 'localhost'
  }

  /**
   * Get or create singleton instance
   * Returns null if hostname routing is not enabled
   */
  static async getInstance(targetResolver: TargetResolver): Promise<HostnameProxyService | null> {
    const config = loadProxyConfig()
    if (!config.useHostnameRouting) {
      return null
    }

    if (!HostnameProxyService.instance) {
      const proxyPort = config.proxyPort || 80

      HostnameProxyService.instance = new HostnameProxyService(
        {
          proxyPort,
          proxyDomain: config.proxyDomain as string, // validated by loadProxyConfig
          protocol: config.proxyProtocol,
        },
        targetResolver,
        config.serverHost,
      )
      await HostnameProxyService.instance.start()
    }

    return HostnameProxyService.instance
  }

  start(): Promise<void> {
    if (this.server) {
      this.serviceLogger.warn('Hostname proxy service is already running')
      return Promise.resolve()
    }

    this.server = http.createServer((req, res) => {
      this.handleRequest(req, res).catch((error) => {
        this.serviceLogger.error({ err: error as Error }, 'Error handling proxy request')
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      })
    })

    return new Promise((resolve, reject) => {
      this.server?.listen(this.config.proxyPort, () => {
        this.serviceLogger.info(
          {
            port: this.config.proxyPort,
            domain: this.config.proxyDomain,
          },
          'Hostname proxy service started',
        )
        resolve()
      })

      this.server?.on('error', (error) => {
        this.serviceLogger.error({ err: error }, 'Failed to start hostname proxy service')
        reject(error)
      })
    })
  }

  stop(): Promise<void> {
    if (!this.server) {
      return Promise.resolve()
    }

    return new Promise((resolve, reject) => {
      this.server?.close((error) => {
        if (error) {
          this.serviceLogger.error({ err: error }, 'Error stopping hostname proxy service')
          reject(error)
        } else {
          this.serviceLogger.info('Hostname proxy service stopped')
          this.server = null
          HostnameProxyService.instance = null
          resolve()
        }
      })
    })
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Auth + subdomain extraction + resolveWithContext branching is inherently sequential
  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const startTime = Date.now()
    const hostname = req.headers.host?.split(':')[0] || ''
    const requestPath = req.url || '/'

    // Health check bypass: respond 200 OK for health probes
    // Health probes typically have IP-based Host headers (not domain names) or hit /health
    const isHealthCheck = requestPath === '/health' || requestPath === '/healthz'
    const isIpBasedHost = /^(\d{1,3}\.){3}\d{1,3}$/.test(hostname)

    if (isHealthCheck || (isIpBasedHost && requestPath === '/')) {
      res.writeHead(200, { 'Content-Type': 'text/plain' })
      res.end('OK')
      return
    }

    // Resolve session (with caching to avoid DB hits on every asset request)
    // Wrap with timeout to prevent hanging on slow DB queries
    const authStart = Date.now()
    let session: Awaited<ReturnType<typeof this.getCachedSession>>
    try {
      session = await withTimeout(
        this.getCachedSession(req),
        PREP_PHASE_TIMEOUT_MS,
        'Session lookup timeout',
      )
    } catch (error) {
      if (error instanceof TimeoutError) {
        this.serviceLogger.error(
          { hostname, durationMs: Date.now() - authStart },
          'Auth phase timeout - session lookup took too long',
        )
        res.writeHead(504, { 'Content-Type': 'text/plain' })
        res.end('Gateway Timeout: Authentication service unavailable')
        return
      }
      throw error
    }
    this.serviceLogger.debug({ durationMs: Date.now() - authStart }, 'Auth phase completed')

    // If session is null, auth is enabled but no valid session found
    if (session === null) {
      this.serviceLogger.info(
        {
          requestUrl: req.url,
          hostname: req.headers.host,
          authProvider: process.env['AUTH_PROVIDER'],
        },
        'Unauthenticated hostname proxy request, returning 401',
      )

      res.writeHead(401, {
        'Content-Type': 'application/json',
      })
      res.end(
        JSON.stringify({
          error: 'Unauthorized',
          message: 'Authentication required. Please log in to access the preview.',
        }),
      )
      return
    }

    const subdomain = extractSubdomain(hostname, this.config.proxyDomain)

    if (!subdomain) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Invalid hostname ${hostname}`)
      return
    }

    // Parse taskId and optional serverName from subdomain (e.g., "task-123--files")
    const { taskId, serverName } = parseSubdomain(subdomain)

    // Wrap target resolution in SessionContext so downstream DB queries
    // (e.g. taskRepo.get, repoService.getRepositoryById) have an authenticated user
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Resolution + timeout handling + two code paths (resolveWithContext vs resolve) is inherently sequential
    const resolveTarget = async () => {
      const resolveStart = Date.now()

      // Delegate to app-specific resolver - prefer resolveWithContext for richer errors
      if (this.targetResolver.resolveWithContext) {
        let result: Awaited<ReturnType<typeof this.targetResolver.resolveWithContext>>
        try {
          result = await withTimeout(
            this.targetResolver.resolveWithContext(taskId, serverName),
            PREP_PHASE_TIMEOUT_MS,
            'Target resolution timeout',
          )
        } catch (error) {
          if (error instanceof TimeoutError) {
            this.serviceLogger.error(
              { subdomain, taskId, serverName, durationMs: Date.now() - resolveStart },
              'Resolution phase timeout - target lookup took too long',
            )
            res.writeHead(504, { 'Content-Type': 'text/plain' })
            res.end('Gateway Timeout: Target resolution service unavailable')
            return
          }
          throw error
        }
        this.serviceLogger.debug(
          { durationMs: Date.now() - resolveStart },
          'Resolution phase completed',
        )

        if (!result.ok) {
          this.serviceLogger.info(
            { subdomain, serverName, hostname, url: req.url, reason: result.reason },
            'Proxy target resolution failed - returning styled error page',
          )

          // Fire-and-forget auto-start if available (e.g. stopped workspace)
          if (result.autoStart) {
            result.autoStart()
          }

          const statusCode = result.reason === 'no_workspace' ? 404 : 503
          const html = renderProxyErrorPage(subdomain, result.reason)
          res.writeHead(statusCode, {
            'Content-Type': 'text/html; charset=utf-8',
            'Cache-Control': 'no-cache, no-store, must-revalidate',
          })
          res.end(html)
          return
        }

        this.serviceLogger.debug(
          { totalPrepMs: Date.now() - startTime },
          'Proxy prep completed, forwarding to target',
        )
        this.proxyToTarget(req, res, result.target, subdomain, taskId)
        return
      }

      // Fallback to simple resolve()
      let target: Awaited<ReturnType<typeof this.targetResolver.resolve>>
      try {
        target = await withTimeout(
          this.targetResolver.resolve(taskId, serverName),
          PREP_PHASE_TIMEOUT_MS,
          'Target resolution timeout',
        )
      } catch (error) {
        if (error instanceof TimeoutError) {
          this.serviceLogger.error(
            { subdomain, taskId, serverName, durationMs: Date.now() - resolveStart },
            'Resolution phase timeout - target lookup took too long',
          )
          res.writeHead(504, { 'Content-Type': 'text/plain' })
          res.end('Gateway Timeout: Target resolution service unavailable')
          return
        }
        throw error
      }
      this.serviceLogger.debug(
        { durationMs: Date.now() - resolveStart },
        'Resolution phase completed',
      )

      if (!target) {
        this.serviceLogger.info(
          { subdomain, hostname, url: req.url },
          'Proxy target resolution failed - returning 404',
        )
        const html = renderProxyErrorPage(subdomain, 'no_workspace')
        res.writeHead(404, {
          'Content-Type': 'text/html; charset=utf-8',
          'Cache-Control': 'no-cache, no-store, must-revalidate',
        })
        res.end(html)
        return
      }

      // Proxy to the dev server (or remote worker pod)
      this.serviceLogger.debug(
        { totalPrepMs: Date.now() - startTime },
        'Proxy prep completed, forwarding to target',
      )
      this.proxyToTarget(req, res, target, subdomain, taskId)
    }

    // session is undefined when auth is disabled, UserSession when authenticated
    if (session) {
      await SessionContext.run(session, resolveTarget)
    } else {
      await resolveTarget()
    }
  }

  /**
   * Get session for a proxy request, using a TTL cache to avoid DB lookups on every asset.
   * Returns UserSession if authenticated, null if auth required but invalid, undefined if auth disabled.
   */
  private async getCachedSession(req: IncomingMessage): Promise<UserSession | null | undefined> {
    const authProvider = process.env['AUTH_PROVIDER']
    if (!authProvider || authProvider === 'disabled') {
      return undefined
    }

    // Build cache key from cookie or auth header
    const cacheKey =
      req.headers.authorization ?? getCookieValue(req.headers.cookie, SESSION_COOKIE_NAME)

    if (cacheKey) {
      const cached = this.sessionCache.get(cacheKey)
      if (cached) {
        return cached
      }
    }

    const session = await getProxyRequestSession(req, this.serviceLogger)

    // Cache successful lookups (LRU cache handles TTL and size limits automatically)
    if (session && cacheKey) {
      this.sessionCache.set(cacheKey, session)
    }

    return session
  }

  /**
   * Proxy request to target (sandbox pod) using Bun's native fetch
   *
   * Uses fetch() for reliable request/response streaming.
   * Sandbox handles all preview logic including HTML injection and compression.
   *
   * On proxy error, lazily queries server status to pick the right error page.
   *
   * Includes:
   * - Request timeout (30s) via AbortController
   * - Client disconnect cleanup via abort signal
   * - Streaming response body back to client
   */
  private proxyToTarget(
    req: IncomingMessage,
    res: ServerResponse,
    target: TargetInfo,
    subdomain: string,
    taskId: string,
  ): void {
    const targetUrl = target.targetUrl
      ? new URL(target.targetUrl)
      : new URL(`http://${this.serverHost}:${target.port}`)

    // Build full URL: targetUrl base + incoming request path
    const basePath = targetUrl.pathname.replace(/\/$/, '')
    const incomingPath = req.url || '/'
    const fullUrl = `${targetUrl.origin}${basePath}${incomingPath}`

    // Build headers for fetch - filter out undefined values and hop-by-hop headers
    const hopByHopHeaders = new Set([
      'connection',
      'keep-alive',
      'proxy-authenticate',
      'proxy-authorization',
      'te',
      'trailers',
      'transfer-encoding',
      'upgrade',
    ])

    const headers: Record<string, string> = {}
    for (const [key, value] of Object.entries(req.headers)) {
      if (value && !hopByHopHeaders.has(key.toLowerCase())) {
        headers[key] = Array.isArray(value) ? value.join(', ') : value
      }
    }
    // Override host and add forwarded headers
    headers['host'] = targetUrl.host
    headers['x-forwarded-host'] = req.headers.host || ''
    headers['x-forwarded-proto'] = this.config.protocol

    // Set up abort controller for timeout and client disconnect
    const controller = new AbortController()
    const timeoutId = setTimeout(() => {
      this.serviceLogger.warn({ subdomain, taskId }, 'Proxy request timeout')
      controller.abort()
    }, PROXY_REQUEST_TIMEOUT_MS)

    // Abort on client disconnect
    req.on('close', () => {
      if (!res.writableEnded) {
        controller.abort()
      }
    })

    // Determine request body - only for methods that can have a body
    const hasBody =
      req.method !== 'GET' &&
      req.method !== 'HEAD' &&
      (req.headers['content-length'] || req.headers['transfer-encoding'])

    // Build fetch options
    const fetchOptions: RequestInit = {
      method: req.method || 'GET',
      headers,
      signal: controller.signal,
    }

    // Add body and duplex only for requests with a body
    if (hasBody) {
      fetchOptions.body = Readable.toWeb(req) as ReadableStream
      // Bun requires duplex: 'half' for streaming request bodies
      ;(fetchOptions as Record<string, unknown>)['duplex'] = 'half'
    }

    // Execute the proxy request
    fetch(fullUrl, fetchOptions)
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Proxy handler has inherent complexity for streaming + error handling
      .then(async (proxyRes) => {
        clearTimeout(timeoutId)

        if (res.headersSent) {
          return
        }

        // Convert Headers to plain object for writeHead
        // Also skip content-encoding and content-length since fetch() auto-decompresses
        const skipResponseHeaders = new Set([
          ...hopByHopHeaders,
          'content-encoding', // fetch() auto-decompresses, so this would be wrong
          'content-length', // length changes after decompression
        ])
        const responseHeaders: Record<string, string> = {}
        proxyRes.headers.forEach((value, key) => {
          if (!skipResponseHeaders.has(key.toLowerCase())) {
            responseHeaders[key] = value
          }
        })

        res.writeHead(proxyRes.status, responseHeaders)

        // Stream response body
        if (proxyRes.body) {
          const reader = proxyRes.body.getReader()
          try {
            while (true) {
              const { done, value } = await reader.read()
              if (done) {
                break
              }
              if (!res.writableEnded) {
                res.write(value)
              }
            }
          } catch (err) {
            this.serviceLogger.warn({ err, subdomain, taskId }, 'Proxy response stream error')
          } finally {
            reader.releaseLock()
          }
        }

        if (!res.writableEnded) {
          res.end()
        }
      })
      .catch((error) => {
        clearTimeout(timeoutId)

        const isTimeout = error?.name === 'AbortError'
        const isClientDisconnect = res.writableEnded

        if (isClientDisconnect) {
          return
        }

        this.serviceLogger.warn(
          { err: error, target: fullUrl, subdomain, taskId, isTimeout },
          isTimeout ? 'Proxy request timeout' : 'Proxy connection failed',
        )

        if (res.headersSent) {
          res.end()
          return
        }

        // Lazy status check — only query sandbox on proxy error
        const statusCode = isTimeout ? 504 : 502
        this.getPreviewStatusForError(taskId, subdomain)
          .then((reason) => {
            const html = reason
              ? renderProxyErrorPage(subdomain, reason)
              : renderBadGatewayPage(subdomain)
            res.writeHead(statusCode, {
              'Content-Type': 'text/html; charset=utf-8',
              'Cache-Control': 'no-cache, no-store, must-revalidate',
            })
            res.end(html)
          })
          .catch(() => {
            const html = renderBadGatewayPage(subdomain)
            res.writeHead(statusCode, {
              'Content-Type': 'text/html; charset=utf-8',
              'Cache-Control': 'no-cache, no-store, must-revalidate',
            })
            res.end(html)
          })
      })
  }

  /**
   * Query server status lazily on proxy error to determine the appropriate error page.
   * Delegates to TargetResolver.getServerStatus if available.
   * Wrapped with timeout to prevent hanging during error handling.
   */
  private async getPreviewStatusForError(
    taskId: string,
    subdomain: string,
  ): Promise<ResolutionFailureReason | null> {
    if (!this.targetResolver.getServerStatus) {
      return null
    }

    try {
      return await withTimeout(
        this.targetResolver.getServerStatus(taskId),
        PREP_PHASE_TIMEOUT_MS,
        'Server status lookup timeout',
      )
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      const isTimeout = error instanceof TimeoutError
      this.serviceLogger.debug(
        { taskId, subdomain, error: errorMessage, isTimeout },
        isTimeout
          ? 'Server status lookup timed out for proxy error page'
          : 'Failed to get server status for proxy error page',
      )
      return null
    }
  }

  getProxyUrl(subdomain: string): string {
    return `${this.config.protocol}://${subdomain}.${this.config.proxyDomain}`
  }
}
