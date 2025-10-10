/**
 * Shared proxy utilities used by both PreviewProxy and HostnameProxy
 */

import type { IncomingMessage, ServerResponse } from 'node:http'
import { createModifiedHtmlHeaders, injectScriptIntoHtml } from './html-injector.js'

/**
 * Handle HTML response with optional script injection
 * Used by both PreviewProxy and HostnameProxy
 */
export function handleHtmlResponse(
  proxyRes: IncomingMessage,
  res: ServerResponse,
  scriptContent?: string,
): void {
  const chunks: Buffer[] = []

  proxyRes.on('data', (chunk) => {
    chunks.push(chunk)
  })

  proxyRes.on('end', () => {
    const html = Buffer.concat(chunks).toString('utf-8')

    // Inject script using shared utility (if provided)
    const modifiedHtml = scriptContent ? injectScriptIntoHtml(html, scriptContent) : html

    // Update headers for modified HTML content
    const headers = createModifiedHtmlHeaders(proxyRes.headers)

    res.writeHead(proxyRes.statusCode || 200, headers)
    res.end(modifiedHtml)
  })

  proxyRes.on('error', () => {
    res.writeHead(500, { 'Content-Type': 'text/plain' })
    res.end('Internal Server Error')
  })
}

/**
 * Check if a content-type header indicates HTML
 */
export function isHtmlResponse(contentType: string): boolean {
  return contentType.includes('text/html')
}

/**
 * Extract subdomain from hostname
 * e.g., 'task-123.proxy.moatless.ai' with domain 'proxy.moatless.ai' → 'task-123'
 */
export function extractSubdomain(hostname: string, domain: string): string | null {
  if (!hostname.endsWith(domain)) {
    return null
  }
  const subdomain = hostname.slice(0, -(domain.length + 1))
  return subdomain || null
}

/**
 * Check if a path is a framework-internal route that should not trigger navigation events
 * These are development/infrastructure requests, not user navigations
 */
export function isFrameworkInternalRoute(path: string): boolean {
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

/**
 * Check if a path is an asset request
 */
export function isAssetRequest(path: string): boolean {
  return /\.(js|css|png|jpg|jpeg|gif|svg|ico|woff|woff2|ttf|eot)$/i.test(path)
}
