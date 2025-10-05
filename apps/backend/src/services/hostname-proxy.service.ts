import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import { createModifiedHtmlHeaders, injectScriptIntoHtml } from '../utils/html-injector.js'
import type { Logger } from '../utils/logger.js'
import { getPreviewInspectorScript } from '../utils/preview-inspector-script.js'
import type { WorkspaceService } from './workspace.service.js'

/**
 * Independent hostname-based proxy service
 * Routes requests based on subdomain to corresponding preview servers
 * Queries WorkspaceService dynamically for preview server info
 */
export class HostnameProxyService {
  private server: http.Server | null = null
  private static instance: HostnameProxyService | null = null
  private readonly logger: Logger
  private readonly proxyPort: number
  private readonly proxyDomain: string
  private readonly protocol: string
  private workspaceService?: WorkspaceService

  private constructor(logger: Logger, proxyPort: number, proxyDomain: string, protocol = 'http') {
    this.logger = logger.child({ service: 'hostname-proxy' })
    this.proxyPort = proxyPort
    this.proxyDomain = proxyDomain
    this.protocol = protocol
  }

  /**
   * Get or create singleton instance
   * Returns null if hostname routing is not enabled
   */
  static async getInstance(logger: Logger): Promise<HostnameProxyService | null> {
    const useHostnameRouting = process.env['USE_HOSTNAME_ROUTING'] === 'true'
    if (!useHostnameRouting) {
      return null
    }

    if (!HostnameProxyService.instance) {
      const proxyPort = Number.parseInt(process.env['PROXY_PORT'] || '80', 10)
      const proxyDomain = process.env['PROXY_DOMAIN'] || 'proxy.moatless.ai'
      const protocol = process.env['PROXY_PROTOCOL'] || 'http'
      HostnameProxyService.instance = new HostnameProxyService(
        logger,
        proxyPort,
        proxyDomain,
        protocol,
      )
      await HostnameProxyService.instance.start()
    }

    return HostnameProxyService.instance
  }

  /**
   * Set workspace service for querying preview server info
   * Must be called after instance creation but before handling requests
   */
  setWorkspaceService(workspaceService: WorkspaceService): void {
    this.workspaceService = workspaceService
  }

  async start(): Promise<void> {
    if (this.server) {
      this.logger.warn('Hostname proxy service is already running')
      return
    }

    this.server = http.createServer((req, res) => {
      this.handleRequest(req, res).catch((error) => {
        this.logger.error({ error }, 'Error handling proxy request')
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      })
    })

    return new Promise((resolve, reject) => {
      this.server!.listen(this.proxyPort, () => {
        this.logger.info(
          { port: this.proxyPort, domain: this.proxyDomain },
          'Hostname proxy service started',
        )
        resolve()
      })

      this.server!.on('error', (error) => {
        this.logger.error({ error }, 'Failed to start hostname proxy service')
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
          this.logger.error({ error }, 'Error stopping hostname proxy service')
          reject(error)
        } else {
          this.logger.info('Hostname proxy service stopped')
          this.server = null
          resolve()
        }
      })
    })
  }

  private extractTaskId(hostname: string): string | null {
    // Extract taskId from hostname like "task-123.proxy.moatless.ai"
    if (!hostname.endsWith(this.proxyDomain)) {
      return null
    }

    const subdomain = hostname.slice(0, -(this.proxyDomain.length + 1))
    return subdomain || null
  }

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const hostname = req.headers.host?.split(':')[0]

    if (!hostname) {
      res.writeHead(400, { 'Content-Type': 'text/plain' })
      res.end('Bad Request: No hostname provided')
      return
    }

    const taskId = this.extractTaskId(hostname)

    if (!taskId) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Invalid hostname ${hostname}`)
      return
    }

    if (!this.workspaceService) {
      this.logger.error('WorkspaceService not set on HostnameProxyService')
      res.writeHead(500, { 'Content-Type': 'text/plain' })
      res.end('Internal Server Error: Service not configured')
      return
    }

    // Query workspace service for preview server info
    const workspace = await this.workspaceService.getWorkspace(taskId)
    if (!workspace) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: No workspace found for task ${taskId}`)
      return
    }

    const serverStatus = await workspace.getServerStatus()
    if (!serverStatus || serverStatus.status !== 'started' || !serverStatus.url) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Preview server not running for task ${taskId}`)
      return
    }

    // Extract port from URL (e.g., "http://localhost:9000" -> 9000)
    const targetUrl = new URL(serverStatus.url)
    const targetPort = Number.parseInt(targetUrl.port, 10)

    if (!targetPort) {
      this.logger.error({ url: serverStatus.url }, 'Could not extract port from preview server URL')
      res.writeHead(500, { 'Content-Type': 'text/plain' })
      res.end('Internal Server Error: Invalid server URL')
      return
    }

    // Get script content for this task
    const repositoryConfig = workspace.environmentConfig
    const enableInspector = repositoryConfig.previewConfig?.enableInspector !== false

    // Proxy to the dev server (not to a per-task proxy, but directly to dev server)
    await this.proxyToDevServer(
      req,
      res,
      targetPort,
      taskId,
      enableInspector ? getPreviewInspectorScript(taskId) : null,
    )
  }

  private async proxyToDevServer(
    req: IncomingMessage,
    res: ServerResponse,
    targetPort: number,
    taskId: string,
    scriptContent: string | null,
  ): Promise<void> {
    const targetUrl = new URL(`http://localhost:${targetPort}`)
    const requestUrl = new URL(req.url || '/', targetUrl)

    // Copy query parameters
    requestUrl.search = new URL(req.url || '/', 'http://localhost').search

    // Remove accept-encoding to prevent compression so we can modify HTML
    const { 'accept-encoding': _removed, ...forwardHeaders } = req.headers

    // Get client IP for x-forwarded-for
    const clientIp = req.socket.remoteAddress || ''
    const existingForwardedFor = req.headers['x-forwarded-for']
    const xForwardedFor = existingForwardedFor ? `${existingForwardedFor}, ${clientIp}` : clientIp

    const options: http.RequestOptions = {
      hostname: targetUrl.hostname,
      port: targetPort,
      path: requestUrl.pathname + requestUrl.search,
      method: req.method,
      headers: {
        ...forwardHeaders,
        host: targetUrl.host,
        'x-forwarded-host': req.headers.host || '',
        'x-forwarded-proto': this.protocol,
        'x-forwarded-for': xForwardedFor,
      },
    }

    const protocol = targetUrl.protocol === 'https:' ? https : http

    const proxyReq = protocol.request(options, (proxyRes) => {
      const contentType = proxyRes.headers['content-type'] || ''
      const isHtml = contentType.includes('text/html')

      // If HTML and we have a script to inject
      if (isHtml && req.method === 'GET' && scriptContent) {
        this.handleHtmlResponse(proxyRes, res, scriptContent)
      } else {
        // Pass through non-HTML responses or when inspector disabled
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.logger.error({ error, targetPort, taskId }, 'Proxy request error')
      res.writeHead(502, { 'Content-Type': 'text/plain' })
      res.end('Bad Gateway')
    })

    // Forward request body if present
    req.pipe(proxyReq)
  }

  private handleHtmlResponse(
    proxyRes: IncomingMessage,
    res: ServerResponse,
    scriptContent: string,
  ): void {
    const chunks: Buffer[] = []

    proxyRes.on('data', (chunk) => {
      chunks.push(chunk)
    })

    proxyRes.on('end', () => {
      const html = Buffer.concat(chunks).toString('utf-8')

      // Inject script using shared utility
      const modifiedHtml = injectScriptIntoHtml(html, scriptContent)

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

  getProxyUrl(taskId: string): string {
    return `${this.protocol}://${taskId}.${this.proxyDomain}`
  }
}
