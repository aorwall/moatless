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
    if (
      value !== undefined &&
      key !== 'content-length' && // Will be incorrect after modification
      key !== 'content-encoding' && // Removed compression
      key !== 'transfer-encoding' && // Converted from chunked to buffered
      key !== 'etag' && // Cache validation
      key !== 'last-modified' && // Cache validation
      key !== 'cache-control' // Will override with no-cache
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
