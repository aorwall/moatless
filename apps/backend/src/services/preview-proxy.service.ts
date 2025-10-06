import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import { createModifiedHtmlHeaders, injectScriptIntoHtml } from '../utils/html-injector.js'
import type { Logger } from '../utils/logger.js'
import type { EventService } from './event.service.js'

interface ProxyConfig {
  targetUrl: string
  proxyPort: number
  scriptContent: string
  logger: Logger
  taskId?: string
  eventService?: EventService
}

export class PreviewProxyService {
  private server: http.Server | null = null
  private readonly config: ProxyConfig
  private readonly logger: Logger
  private currentPath = '/'

  constructor(config: ProxyConfig) {
    this.config = config
    this.logger = config.logger.child({ service: 'preview-proxy' })
  }

  async start(): Promise<void> {
    if (this.server) {
      throw new Error('Proxy server is already running')
    }

    this.server = http.createServer((req, res) => {
      this.handleRequest(req, res).catch((error) => {
        this.logger.error({ error }, 'Error handling proxy request')
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      })
    })

    return new Promise((resolve, reject) => {
      this.server!.listen(this.config.proxyPort, () => {
        this.logger.info(
          { port: this.config.proxyPort, target: this.config.targetUrl },
          'Preview proxy server started',
        )
        resolve()
      })

      this.server!.on('error', (error) => {
        this.logger.error({ error }, 'Failed to start proxy server')
        reject(error)
      })
    })
  }

  async stop(): Promise<void> {
    if (!this.server) {
      return
    }

    return new Promise((resolve, reject) => {
      this.server!.close((error) => {
        if (error) {
          this.logger.error({ error }, 'Error stopping proxy server')
          reject(error)
        } else {
          this.logger.info('Preview proxy server stopped')
          this.server = null
          resolve()
        }
      })
    })
  }

  /**
   * Check if a path is a framework-internal route that should not trigger navigation events
   * These are development/infrastructure requests, not user navigations
   */
  private isFrameworkInternalRoute(path: string): boolean {
    // Next.js internal routes
    if (path.startsWith('/_next/') || path.startsWith('/__nextjs_')) {
      return true
    }

    // Vite development routes
    if (
      path.startsWith('/@vite/') ||
      path.startsWith('/@react-refresh') ||
      path.startsWith('/__vite_')
    ) {
      return true
    }

    // Webpack HMR and other dev server routes
    if (path.includes('/webpack-hmr') || path.includes('/hot-update')) {
      return true
    }

    return false
  }

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const targetUrl = new URL(this.config.targetUrl)
    const requestUrl = new URL(req.url || '/', targetUrl)

    // Copy query parameters
    requestUrl.search = new URL(req.url || '/', 'http://localhost').search

    // Track navigation for HTML GET requests
    if (req.method === 'GET') {
      const path = requestUrl.pathname
      // Only track actual page navigation, not asset requests or framework-internal routes
      const isAssetRequest = /\.(js|css|png|jpg|jpeg|gif|svg|ico|woff|woff2|ttf|eot)$/i.test(path)
      const isInternalRoute = this.isFrameworkInternalRoute(path)

      if (!(isAssetRequest || isInternalRoute) && this.currentPath !== path) {
        const previousPath = this.currentPath
        this.currentPath = path

        this.logger.info('Navigation detected', {
          from: previousPath,
          to: path,
          taskId: this.config.taskId,
        })

        // Emit navigation event if event service is available
        if (this.config.eventService && this.config.taskId) {
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
      const isHtml = contentType.includes('text/html')

      if (isHtml && req.method === 'GET') {
        // Intercept HTML responses to inject script
        this.handleHtmlResponse(proxyRes, res)
      } else {
        // Pass through non-HTML responses
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.logger.error({ error, url: requestUrl.href }, 'Proxy request error')
      res.writeHead(502, { 'Content-Type': 'text/plain' })
      res.end('Bad Gateway')
    })

    // Forward request body if present
    req.pipe(proxyReq)
  }

  private handleHtmlResponse(proxyRes: IncomingMessage, res: ServerResponse): void {
    const chunks: Buffer[] = []

    proxyRes.on('data', (chunk) => {
      chunks.push(chunk)
    })

    proxyRes.on('end', () => {
      const html = Buffer.concat(chunks).toString('utf-8')

      // Inject script using shared utility
      const modifiedHtml = injectScriptIntoHtml(html, this.config.scriptContent)

      // Update headers for modified HTML content
      const headers = createModifiedHtmlHeaders(proxyRes.headers)

      res.writeHead(proxyRes.statusCode || 200, headers)
      res.end(modifiedHtml)
    })

    proxyRes.on('error', (error) => {
      this.logger.error({ error }, 'Error reading HTML response')
      res.writeHead(500, { 'Content-Type': 'text/plain' })
      res.end('Internal Server Error')
    })
  }

  getProxyUrl(): string {
    return `http://localhost:${this.config.proxyPort}`
  }

  getCurrentPath(): string {
    return this.currentPath
  }

  private async emitNavigationEvent(from: string, to: string): Promise<void> {
    if (!(this.config.eventService && this.config.taskId)) {
      return
    }

    try {
      await this.config.eventService.emitEvent({
        eventId: `nav-${Date.now()}`,
        type: 'navigation',
        subType: 'navigated',
        taskId: this.config.taskId,
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
  taskId?: string,
  eventService?: EventService,
): PreviewProxyService {
  return new PreviewProxyService({
    targetUrl,
    proxyPort,
    scriptContent,
    logger,
    ...(taskId !== undefined && { taskId }),
    ...(eventService !== undefined && { eventService }),
  })
}
