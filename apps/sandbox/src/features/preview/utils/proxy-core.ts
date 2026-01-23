/**
 * Core proxy utilities for preview server
 * Pure helper functions with no external dependencies
 */

/**
 * Checks if the request is for a static asset (CSS, JS, images, fonts, etc.)
 */
export function isAssetRequest(path: string): boolean {
  const assetExtensions = [
    '.css',
    '.js',
    '.mjs',
    '.png',
    '.jpg',
    '.jpeg',
    '.gif',
    '.svg',
    '.ico',
    '.woff',
    '.woff2',
    '.ttf',
    '.eot',
    '.json',
    '.map',
  ]
  return assetExtensions.some((ext) => path.endsWith(ext))
}

/**
 * Checks if the response is HTML based on content-type header
 */
export function isHtmlResponse(contentType: string | null | undefined): boolean {
  return !!contentType && contentType.includes('text/html')
}

/**
 * Checks if request path is a framework-specific internal route
 * These are excluded from HTML injection to avoid breaking HMR/websockets
 */
export function isFrameworkInternalRoute(path: string): boolean {
  const internalPrefixes = [
    '/__vite',
    '/@vite',
    '/_next',
    '/__next',
    '/@fs',
    '/__webpack',
    '/_webpack',
    '/hot-update',
  ]
  return internalPrefixes.some((prefix) => path.startsWith(prefix))
}

/**
 * Extract subdomain from hostname relative to a base domain
 * e.g., 'task-123.proxy.moatless.xyz' with domain 'proxy.moatless.xyz' → 'task-123'
 * Returns null if hostname doesn't end with the specified domain
 */
export function extractSubdomain(hostname: string, domain: string): string | null {
  if (!hostname.endsWith(domain)) {
    return null
  }
  const subdomain = hostname.slice(0, -(domain.length + 1))
  return subdomain || null
}

/**
 * Handle HTML response by injecting script if conditions are met
 * Returns modified HTML if injection occurred, original body otherwise
 */
export function handleHtmlResponse(
  body: string,
  contentType: string | null | undefined,
  path: string,
  scriptContent: string | undefined,
  injectScriptFn: (html: string, script: string) => string,
): string {
  // Only inject script in HTML responses that aren't framework internals
  if (
    scriptContent &&
    isHtmlResponse(contentType) &&
    !isFrameworkInternalRoute(path) &&
    !isAssetRequest(path)
  ) {
    return injectScriptFn(body, scriptContent)
  }
  return body
}
