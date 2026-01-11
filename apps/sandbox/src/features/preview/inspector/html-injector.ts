/**
 * Utility for injecting scripts into HTML content
 */

/**
 * Injects a script tag into HTML content
 * Tries to inject before </body>, then </html>, or appends to end
 */
export function injectScriptIntoHtml(html: string, scriptContent: string): string {
  const scriptTag = `<script>${scriptContent}</script>`
  const injectionPoint = '</body>'

  if (html.includes(injectionPoint)) {
    return html.replace(injectionPoint, `${scriptTag}${injectionPoint}`)
  }
  if (html.includes('</html>')) {
    return html.replace('</html>', `${scriptTag}</html>`)
  }
  // Fallback: append to end
  return html + scriptTag
}

/**
 * Creates headers for modified HTML content
 * Removes headers that would cause caching issues and adds cache-busting headers
 */
export function createModifiedHtmlHeaders(
  originalHeaders: Record<string, string | string[] | undefined>,
): Record<string, string | string[]> {
  const headers: Record<string, string | string[]> = {}

  // Copy headers except those that would cause issues with modified content
  for (const [key, value] of Object.entries(originalHeaders)) {
    const lowerKey = key.toLowerCase()
    if (
      value !== undefined &&
      lowerKey !== 'content-length' && // Will be incorrect after modification
      lowerKey !== 'content-encoding' && // Removed compression
      lowerKey !== 'transfer-encoding' && // Converted from chunked to buffered
      lowerKey !== 'etag' && // Cache validation
      lowerKey !== 'last-modified' && // Cache validation
      lowerKey !== 'cache-control' && // Will override with no-cache
      lowerKey !== 'x-frame-options' && // Allow iframe embedding
      lowerKey !== 'content-security-policy' // Allow iframe embedding
    ) {
      headers[key] = value
    }
  }

  // Add cache-busting headers for modified HTML content
  headers['cache-control'] = 'no-cache, no-store, must-revalidate'
  headers['pragma'] = 'no-cache'
  headers['expires'] = '0'

  return headers
}
