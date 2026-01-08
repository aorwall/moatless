import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger.js'
import { loadProxyConfig } from '../config/proxy-config.js'
import type { HostnameProxyConfig, TargetInfo, TargetResolver } from '../types/index.js'
import { isProxyRequestAuthorized } from '../utils/proxy-auth.utils.js'
import { extractSubdomain, handleHtmlResponseStream, isHtmlResponse } from '../utils/proxy-core.js'

/**
 * HostnameProxyService - Independent hostname-based proxy service
 *
 * Routes requests based on subdomain to corresponding preview servers.
 * Uses TargetResolver interface to query for target server information.
 *
 * Example: task-123.proxy.moatless.xyz → routes to task-123's preview server
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

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
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

    const hostname = req.headers.host?.split(':')[0]

    if (!hostname) {
      res.writeHead(400, { 'Content-Type': 'text/plain' })
      res.end('Bad Request: No hostname provided')
      return
    }

    const subdomain = extractSubdomain(hostname, this.config.proxyDomain)

    if (!subdomain) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Invalid hostname ${hostname}`)
      return
    }

    // Delegate to app-specific resolver
    const target = await this.targetResolver.resolve(subdomain)

    if (!target) {
      this.serviceLogger.info(
        {
          subdomain,
          hostname,
          url: req.url,
        },
        'Proxy target resolution failed - returning 404',
      )
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: No preview server found for ${subdomain}`)
      return
    }

    // Proxy to the dev server (or remote worker pod)
    this.proxyToTarget(req, res, target)
  }

  private proxyToTarget(req: IncomingMessage, res: ServerResponse, target: TargetInfo): void {
    // Use targetUrl if provided (Kubernetes worker pod), otherwise use localhost:port
    const targetUrl = target.targetUrl
      ? new URL(target.targetUrl)
      : new URL(`http://${this.serverHost}:${target.port}`)

    // Combine targetUrl base path with incoming request path
    // targetUrl may have a path like /server/proxy that must be preserved
    const targetBasePath = targetUrl.pathname.replace(/\/$/, '') // remove trailing slash
    const incomingPath = req.url || '/'
    const combinedPath = incomingPath.startsWith('/')
      ? targetBasePath + incomingPath
      : `${targetBasePath}/${incomingPath}`

    // Parse query string from incoming request
    const incomingUrl = new URL(incomingPath, `http://${this.serverHost}`)
    const requestUrl = new URL(combinedPath, targetUrl.origin)
    requestUrl.search = incomingUrl.search

    // Remove accept-encoding to prevent compression so we can modify HTML
    const { 'accept-encoding': _removed, ...forwardHeaders } = req.headers

    // Get client IP for x-forwarded-for
    const clientIp = req.socket.remoteAddress || ''
    const existingForwardedFor = req.headers['x-forwarded-for']
    const xForwardedFor = existingForwardedFor ? `${existingForwardedFor}, ${clientIp}` : clientIp

    const targetPort = targetUrl.port || (targetUrl.protocol === 'https:' ? 443 : 80)

    const options: http.RequestOptions = {
      hostname: targetUrl.hostname,
      port: targetPort,
      path: requestUrl.pathname + requestUrl.search,
      method: req.method,
      headers: {
        ...forwardHeaders,
        host: targetUrl.host,
        'x-forwarded-host': req.headers.host || '',
        'x-forwarded-proto': this.config.protocol,
        'x-forwarded-for': xForwardedFor,
      },
    }

    const protocol = targetUrl.protocol === 'https:' ? https : http

    const proxyReq = protocol.request(options, (proxyRes) => {
      const contentType = proxyRes.headers['content-type'] || ''
      const isHtml = isHtmlResponse(contentType)

      // If HTML and we have a script to inject
      if (isHtml && req.method === 'GET' && target.scriptContent) {
        handleHtmlResponseStream(proxyRes, res, target.scriptContent)
      } else {
        // Pass through non-HTML responses or when inspector disabled
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.serviceLogger.error(
        {
          err: error,
          targetUrl: targetUrl.toString(),
        },
        'Proxy request error',
      )
      res.writeHead(502, { 'Content-Type': 'text/plain' })
      res.end('Bad Gateway')
    })

    // Forward request body if present
    req.pipe(proxyReq)
  }

  getProxyUrl(subdomain: string): string {
    return `${this.config.protocol}://${subdomain}.${this.config.proxyDomain}`
  }
}
