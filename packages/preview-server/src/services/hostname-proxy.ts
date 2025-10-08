import type { IncomingMessage, ServerResponse } from 'node:http'
import http from 'node:http'
import https from 'node:https'
import { URL } from 'node:url'
import { ProxyCore } from '../core/proxy-core.js'
import type { HostnameProxyConfig, Logger, TargetResolver } from '../types/index.js'

/**
 * Independent hostname-based proxy service
 * Routes requests based on subdomain to corresponding preview servers
 * Uses TargetResolver interface to query for target server information
 */
export class HostnameProxyService {
  private server: http.Server | null = null
  private static instance: HostnameProxyService | null = null

  private constructor(
    private config: HostnameProxyConfig,
    private logger: Logger,
    private targetResolver: TargetResolver,
  ) {
    this.logger = logger.child({ service: 'hostname-proxy' })
  }

  /**
   * Get or create singleton instance
   * Returns null if hostname routing is not enabled
   */
  static async getInstance(
    logger: Logger,
    targetResolver: TargetResolver,
  ): Promise<HostnameProxyService | null> {
    const useHostnameRouting = process.env['USE_HOSTNAME_ROUTING'] === 'true'
    if (!useHostnameRouting) {
      return null
    }

    if (!HostnameProxyService.instance) {
      const proxyPort = Number.parseInt(process.env['PROXY_PORT'] || '80', 10)
      const proxyDomain = process.env['PROXY_DOMAIN'] || 'proxy.moatless.ai'
      const protocol = (process.env['PROXY_PROTOCOL'] || 'http') as 'http' | 'https'

      HostnameProxyService.instance = new HostnameProxyService(
        {
          proxyPort,
          proxyDomain,
          protocol,
        },
        logger,
        targetResolver,
      )
      await HostnameProxyService.instance.start()
    }

    return HostnameProxyService.instance
  }

  start(): Promise<void> {
    if (this.server) {
      this.logger.warn('Hostname proxy service is already running')
      return Promise.resolve()
    }

    this.server = http.createServer((req, res) => {
      this.handleRequest(req, res).catch((error) => {
        this.logger.error('Error handling proxy request', error as Error)
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('Internal Server Error')
      })
    })

    return new Promise((resolve, reject) => {
      this.server?.listen(this.config.proxyPort, () => {
        this.logger.info('Hostname proxy service started', {
          port: this.config.proxyPort,
          domain: this.config.proxyDomain,
        })
        resolve()
      })

      this.server?.on('error', (error) => {
        this.logger.error('Failed to start hostname proxy service', error)
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
          this.logger.error('Error stopping hostname proxy service', error)
          reject(error)
        } else {
          this.logger.info('Hostname proxy service stopped')
          this.server = null
          resolve()
        }
      })
    })
  }

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const hostname = req.headers.host?.split(':')[0]

    if (!hostname) {
      res.writeHead(400, { 'Content-Type': 'text/plain' })
      res.end('Bad Request: No hostname provided')
      return
    }

    const subdomain = ProxyCore.extractSubdomain(hostname, this.config.proxyDomain)

    if (!subdomain) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: Invalid hostname ${hostname}`)
      return
    }

    // Delegate to app-specific resolver
    const target = await this.targetResolver.resolve(subdomain)

    if (!target) {
      res.writeHead(404, { 'Content-Type': 'text/plain' })
      res.end(`Not Found: No preview server found for ${subdomain}`)
      return
    }

    // Proxy to the dev server
    this.proxyToTarget(req, res, target.port, target.scriptContent)
  }

  private proxyToTarget(
    req: IncomingMessage,
    res: ServerResponse,
    targetPort: number,
    scriptContent?: string,
  ): void {
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
        'x-forwarded-proto': this.config.protocol,
        'x-forwarded-for': xForwardedFor,
      },
    }

    const protocol = targetUrl.protocol === 'https:' ? https : http

    const proxyReq = protocol.request(options, (proxyRes) => {
      const contentType = proxyRes.headers['content-type'] || ''
      const isHtml = ProxyCore.isHtmlResponse(contentType)

      // If HTML and we have a script to inject
      if (isHtml && req.method === 'GET' && scriptContent) {
        ProxyCore.handleHtmlResponse(proxyRes, res, scriptContent)
      } else {
        // Pass through non-HTML responses or when inspector disabled
        res.writeHead(proxyRes.statusCode || 200, proxyRes.headers)
        proxyRes.pipe(res)
      }
    })

    proxyReq.on('error', (error) => {
      this.logger.error('Proxy request error', error, {
        targetPort,
      })
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
