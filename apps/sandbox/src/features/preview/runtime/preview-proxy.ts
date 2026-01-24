/**
 * PreviewProxy - Stateless HTTP proxy for preview server requests
 *
 * Proxies requests to the local preview server port and injects
 * the inspector script into HTML responses. No process lifecycle
 * awareness needed - just checks if the port responds.
 */

import http from 'node:http'
import { logger } from '../../../shared/logger'
import { injectScriptIntoHtml } from '../inspector/html-injector'
import { getPreviewInspectorScript } from '../inspector/inspector-script'

const proxyLogger = logger.child({ service: 'preview-proxy' })

/**
 * Handle a proxied preview request.
 * Uses raw HTTP pipe for non-HTML assets (preserves content-encoding).
 * Only intercepts HTML index pages for inspector script injection.
 */
export async function handleProxyRequest(
  req: Request,
  path: string,
  serverId: string,
  port: number,
): Promise<Response> {
  const url = `http://localhost:${port}${path}`
  const isIndex = path === '/' || path.endsWith('/index.html')

  // Only intercept potential HTML index pages for inspector script injection
  if (req.method === 'GET' && isIndex) {
    try {
      const requestHeaders = new Headers(req.headers)
      requestHeaders.delete('accept-encoding')
      requestHeaders.delete('host')

      const response = await fetch(url, {
        method: 'GET',
        headers: requestHeaders,
      })

      const contentType = response.headers.get('content-type') || ''
      if (contentType.includes('text/html')) {
        const html = await response.text()
        const scriptContent = getPreviewInspectorScript(serverId)
        const modified = injectScriptIntoHtml(html, scriptContent)

        const headers = new Headers()
        response.headers.forEach((v, k) => {
          if (
            ![
              'transfer-encoding',
              'connection',
              'keep-alive',
              'content-length',
              'content-encoding',
            ].includes(k.toLowerCase())
          ) {
            headers.set(k, v)
          }
        })
        headers.delete('etag')
        headers.delete('last-modified')
        headers.set('cache-control', 'no-cache, no-store, must-revalidate')
        headers.delete('x-frame-options')
        headers.delete('content-security-policy')

        return new Response(modified, { status: response.status, headers })
      }
      // Not actually HTML - fall through to raw pipe
    } catch (error) {
      proxyLogger.debug({ err: error, path }, 'HTML interception failed, falling back to raw pipe')
    }
  }

  // For all non-HTML: pipe raw bytes unchanged using node:http
  return rawPipe(req, url)
}

/**
 * Pipe request/response bytes unchanged using node:http.
 * Preserves content-encoding so the browser can decode compressed assets correctly.
 */
function rawPipe(req: Request, targetUrl: string): Promise<Response> {
  return new Promise((resolve, reject) => {
    const parsed = new URL(targetUrl)
    const options: http.RequestOptions = {
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname + parsed.search,
      method: req.method,
      headers: { ...Object.fromEntries(req.headers.entries()), host: parsed.host },
    }

    const proxyReq = http.request(options, (proxyRes) => {
      // Convert node:http headers to web Headers
      const headers = new Headers()
      for (const [key, value] of Object.entries(proxyRes.headers)) {
        if (value != null) {
          if (Array.isArray(value)) {
            for (const v of value) {
              headers.append(key, v)
            }
          } else {
            headers.set(key, value)
          }
        }
      }

      // Stream response body as ReadableStream
      const body = new ReadableStream({
        start(controller) {
          proxyRes.on('data', (chunk: Buffer) => controller.enqueue(new Uint8Array(chunk)))
          proxyRes.on('end', () => controller.close())
          proxyRes.on('error', (err) => controller.error(err))
        },
      })

      resolve(new Response(body, { status: proxyRes.statusCode || 200, headers }))
    })

    proxyReq.on('error', (err) => {
      reject(err)
    })

    // Pipe request body if present
    if (req.body && req.method !== 'GET' && req.method !== 'HEAD') {
      const reader = req.body.getReader()
      const pump = async () => {
        while (true) {
          const { done, value } = await reader.read()
          if (done) {
            proxyReq.end()
            break
          }
          proxyReq.write(value)
        }
      }
      pump().catch(() => proxyReq.destroy())
    } else {
      proxyReq.end()
    }
  })
}
