import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import { URL } from 'node:url'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { loadProxyConfig } from '../config/proxy-config.js'
import type { HostnameProxyConfig, TargetInfo, TargetResolver } from '../types/index.js'
import { renderBadGatewayPage, renderProxyErrorPage } from '../utils/error-pages.js'
import { isProxyRequestAuthorized } from '../utils/proxy-auth.utils.js'
import { extractSubdomain } from '../utils/proxy-core.js'

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
@injectable()
export class HostnameProxyService {
  private server: http.Server | null = null
  private readonly serviceLogger: typeof logger
  private static instance: HostnameProxyService | null = null
  private readonly serverHost: string

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

    // Check authentication if enabled
    const authProvider = process.env['AUTH_PROVIDER']
    if (authProvider && authProvider !== 'disabled') {
      const isAuthorized = await isProxyRequestAuthorized(req, this.serviceLogger)

      if (!isAuthorized) {
        // Authentication required but no valid session - return 401 Unauthorized
        this.serviceLogger.info(
          {
            requestUrl: req.url,
            hostname: req.headers.host,
            authProvider,
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
    }

    const subdomain = extractSubdomain(hostname, this.config.proxyDomain)

    if (!subdomain) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Invalid hostname ${hostname}`)
      return
    }

    // Delegate to app-specific resolver - prefer resolveWithContext for richer errors
    if (this.targetResolver.resolveWithContext) {
      const result = await this.targetResolver.resolveWithContext(subdomain)

      if (!result.ok) {
        this.serviceLogger.info(
          { subdomain, hostname, url: req.url, reason: result.reason },
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

      this.proxyToTarget(req, res, result.target, subdomain)
      return
    }

    // Fallback to simple resolve()
    const target = await this.targetResolver.resolve(subdomain)

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
    this.proxyToTarget(req, res, target, subdomain)
  }

  /**
   * Proxy request to target (sandbox pod)
   *
   * Minimal overhead proxy - just pipes request/response through.
   * Sandbox handles all preview logic including HTML injection and compression.
   */
  private proxyToTarget(
    req: IncomingMessage,
    res: ServerResponse,
    target: TargetInfo,
    subdomain?: string,
  ): void {
    const targetUrl = target.targetUrl
      ? new URL(target.targetUrl)
      : new URL(`http://${this.serverHost}:${target.port}`)

    // Build path: targetUrl base path + incoming request path
    const basePath = targetUrl.pathname.replace(/\/$/, '')
    const incomingPath = req.url || '/'
    const fullPath = basePath + incomingPath

    const options: http.RequestOptions = {
      hostname: targetUrl.hostname,
      port: targetUrl.port || 80,
      path: fullPath,
      method: req.method,
      headers: {
        ...req.headers,
        host: targetUrl.host,
        'x-forwarded-host': req.headers.host || '',
        'x-forwarded-proto': this.config.protocol,
      },
    }

    const proxyReq = http.request(options, (proxyRes) => {
      res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
      proxyRes.pipe(res)
    })

    proxyReq.on('error', (error) => {
      this.serviceLogger.error({ err: error, target: targetUrl.href }, 'Proxy error')
      if (!res.headersSent) {
        const html = renderBadGatewayPage(subdomain || 'unknown')
        res.writeHead(502, {
          'Content-Type': 'text/html; charset=utf-8',
          'Cache-Control': 'no-cache, no-store, must-revalidate',
        })
        res.end(html)
      } else {
        res.end()
      }
    })

    req.pipe(proxyReq)
  }

  getProxyUrl(subdomain: string): string {
    return `${this.config.protocol}://${subdomain}.${this.config.proxyDomain}`
  }
}
