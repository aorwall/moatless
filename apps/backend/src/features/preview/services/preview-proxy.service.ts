import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import { logger } from '../../../utils/logger.js'
import type { ProxyConfig } from '../types/index.js'
import { isProxyRequestAuthorized } from '../utils/proxy-auth.utils.js'
import { handleHtmlResponseStream, isHtmlResponse } from '../utils/proxy-core.js'

export class PreviewProxyService {
  private server: http.Server | null = null
  private readonly config: ProxyConfig
  private readonly serviceLogger: typeof logger
  private readonly serverHost: string
  private connections = new Set<http.ServerResponse>()

  constructor(config: ProxyConfig) {
    this.config = config
    this.serverHost = config.serverHost || 'localhost'
    this.serviceLogger = logger.child({ service: 'preview-proxy', serverId: config.serverId })
  }

  start(): Promise<void> {
    if (this.server) {
      throw new Error('Proxy server is already running')
    }

    this.server = http.createServer((req, res) => {
      // Track the connection
      this.connections.add(res)
      res.on('close', () => {
        this.connections.delete(res)
      })

      // Handle async request processing
      this.handleRequest(req, res).catch((error) => {
        this.serviceLogger.error({ err: error as Error }, 'Error handling proxy request')
        if (!res.headersSent) {
          res.writeHead(500, { 'Content-Type': 'text/plain' })
          res.end('Internal Server Error')
        }
      })
    })

    return new Promise((resolve, reject) => {
      this.server?.listen(this.config.proxyPort, () => {
        this.serviceLogger.info(
          {
            port: this.config.proxyPort,
            target: this.config.targetUrl,
          },
          'Preview proxy server started',
        )
        resolve()
      })

      this.server?.on('error', (error) => {
        this.serviceLogger.error({ err: error }, 'Failed to start proxy server')
        reject(error)
      })
    })
  }

  stop(): Promise<void> {
    if (!this.server) {
      this.serviceLogger.info('Preview proxy server already stopped')
      return Promise.resolve()
    }

    // Forcefully close all active connections
    for (const connection of this.connections) {
      connection.destroy()
    }
    this.connections.clear()

    return new Promise((resolve, reject) => {
      // Set a timeout to prevent hanging indefinitely
      const timeout = setTimeout(() => {
        this.serviceLogger.warn('Proxy server close timeout, forcing cleanup')
        this.server = null
        resolve()
      }, 5000)

      this.server?.close((error) => {
        clearTimeout(timeout)
        if (error) {
          this.serviceLogger.error({ err: error }, 'Error stopping proxy server')
          reject(error)
        } else {
          this.serviceLogger.info('Preview proxy server stopped')
          this.server = null
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
            authProvider,
          },
          'Unauthenticated proxy request, returning 401',
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

    const targetUrl = new URL(this.config.targetUrl)
    const requestUrl = new URL(req.url || '/', targetUrl)

    // Copy query parameters
    requestUrl.search = new URL(req.url || '/', `http://${this.serverHost}`).search

    // Remove accept-encoding to prevent compression so we can modify HTML
    const { 'accept-encoding': _removed, ...forwardHeaders } = req.headers

    const options: http.RequestOptions = {
      hostname: targetUrl.hostname,
      port: targetUrl.port || (targetUrl.protocol === 'https:' ? 443 : 80),
      path: requestUrl.pathname + requestUrl.search,
      method: req.method,
      headers: {
        ...forwardHeaders,
        host: targetUrl.host, // Override host header
      },
    }

    const protocol = targetUrl.protocol === 'https:' ? https : http

    const proxyReq = protocol.request(options, (proxyRes) => {
      // Check if this is an HTML response
      const contentType = proxyRes.headers['content-type'] || ''
      const isHtml = isHtmlResponse(contentType)

      if (isHtml && req.method === 'GET') {
        // Intercept HTML responses to inject script
        handleHtmlResponseStream(proxyRes, res, this.config.scriptContent)
      } else {
        // Pass through non-HTML responses
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.serviceLogger.error({ err: error, url: requestUrl.href }, 'Proxy request error')
      res.writeHead(502, { 'Content-Type': 'text/plain' })
      res.end('Bad Gateway')
    })

    // Forward request body if present
    req.pipe(proxyReq)
  }

  getProxyUrl(): string {
    return `http://${this.serverHost}:${this.config.proxyPort}`
  }

  getCurrentPath(): string | undefined {
    // Navigation tracking moved to client-side inspector script
    // This method kept for compatibility but returns undefined
    return undefined
  }
}

export function createPreviewProxy(
  targetUrl: string,
  scriptContent: string,
  proxyPort: number,
  serverId?: string,
  serverHost?: string,
): PreviewProxyService {
  return new PreviewProxyService({
    targetUrl,
    proxyPort,
    scriptContent,
    serverId,
    serverHost,
  })
}
