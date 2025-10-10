import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import {
  handleHtmlResponse,
  isAssetRequest,
  isFrameworkInternalRoute,
  isHtmlResponse,
} from '../core/proxy-core.js'
import type { EventService, Logger, ProxyConfig } from '../types/index.js'

export class PreviewProxyService {
  private server: http.Server | null = null
  private readonly config: ProxyConfig
  private readonly logger: Logger
  private currentPath = '/'
  private connections = new Set<http.ServerResponse>()

  constructor(
    config: ProxyConfig,
    private eventService?: EventService,
  ) {
    this.config = config
    this.logger = (config.logger as Logger) || this.createNoopLogger()
  }

  private createNoopLogger(): Logger {
    const noop = (): void => {
      // Intentionally empty - noop logger does nothing
    }
    return {
      info: noop,
      error: noop,
      debug: noop,
      warn: noop,
      child: (): Logger => this.createNoopLogger(),
    }
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

      try {
        this.handleRequest(req, res)
      } catch (error) {
        this.logger.error('Error handling proxy request', error as Error)
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      }
    })

    return new Promise((resolve, reject) => {
      this.server?.listen(this.config.proxyPort, () => {
        this.logger.info('Preview proxy server started', {
          port: this.config.proxyPort,
          target: this.config.targetUrl,
        })
        resolve()
      })

      this.server?.on('error', (error) => {
        this.logger.error('Failed to start proxy server', error)
        reject(error)
      })
    })
  }

  stop(): Promise<void> {
    if (!this.server) {
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
        this.logger.warn('Proxy server close timeout, forcing cleanup')
        this.server = null
        resolve()
      }, 5000)

      this.server?.close((error) => {
        clearTimeout(timeout)
        if (error) {
          this.logger.error('Error stopping proxy server', error)
          reject(error)
        } else {
          this.logger.info('Preview proxy server stopped')
          this.server = null
          resolve()
        }
      })
    })
  }

  private handleRequest(req: IncomingMessage, res: ServerResponse): void {
    const targetUrl = new URL(this.config.targetUrl)
    const requestUrl = new URL(req.url || '/', targetUrl)

    // Copy query parameters
    requestUrl.search = new URL(req.url || '/', 'http://localhost').search

    // Track navigation for HTML GET requests
    if (req.method === 'GET') {
      const path = requestUrl.pathname
      // Only track actual page navigation, not asset requests or framework-internal routes
      const isAsset = isAssetRequest(path)
      const isInternalRoute = isFrameworkInternalRoute(path)

      if (!(isAsset || isInternalRoute) && this.currentPath !== path) {
        const previousPath = this.currentPath
        this.currentPath = path

        this.logger.info('Navigation detected', {
          from: previousPath,
          to: path,
          serverId: this.config.serverId,
        })

        // Emit navigation event if event service is available
        if (this.eventService && this.config.serverId) {
          this.emitNavigationEvent(previousPath, path).catch((error) => {
            this.logger.error('Failed to emit navigation event', error as Error)
          })
        }
      }
    }

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
        handleHtmlResponse(proxyRes, res, this.config.scriptContent)
      } else {
        // Pass through non-HTML responses
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.logger.error('Proxy request error', error, {
        url: requestUrl.href,
      })
      res.writeHead(502, { 'Content-Type': 'text/plain' })
      res.end('Bad Gateway')
    })

    // Forward request body if present
    req.pipe(proxyReq)
  }

  getProxyUrl(): string {
    return `http://localhost:${this.config.proxyPort}`
  }

  getCurrentPath(): string {
    return this.currentPath
  }

  private async emitNavigationEvent(from: string, to: string): Promise<void> {
    if (!(this.eventService && this.config.serverId)) {
      return
    }

    try {
      await this.eventService.emitEvent({
        eventId: `nav-${Date.now()}`,
        type: 'navigation',
        subType: 'navigated',
        serverId: this.config.serverId,
        timestamp: new Date(),
        isError: false,
        payload: {
          path: to,
          previousPath: from,
          url: `${this.getProxyUrl()}${to}`,
          timestamp: new Date(),
        },
      })
    } catch (error) {
      this.logger.error('Failed to emit navigation event', error as Error)
    }
  }
}

export function createPreviewProxy(
  targetUrl: string,
  scriptContent: string,
  logger: Logger,
  proxyPort: number,
  serverId?: string,
  eventService?: EventService,
): PreviewProxyService {
  return new PreviewProxyService(
    {
      targetUrl,
      proxyPort,
      scriptContent,
      serverId,
      logger,
    },
    eventService,
  )
}
