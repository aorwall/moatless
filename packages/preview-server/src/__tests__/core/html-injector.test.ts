import { describe, expect, it } from 'vitest'
import { createModifiedHtmlHeaders, injectScriptIntoHtml } from '../../core/html-injector.js'

describe('injectScriptIntoHtml', () => {
  const testScript = 'console.log("test")'

  it('should inject script before </body> tag', () => {
    const html = '<html><body>Content</body></html>'
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toBe('<html><body>Content<script>console.log("test")</script></body></html>')
    expect(result).toContain('<script>console.log("test")</script></body>')
  })

  it('should inject script before </html> tag when no </body> tag', () => {
    const html = '<html>No body tag</html>'
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toBe('<html>No body tag<script>console.log("test")</script></html>')
    expect(result).toContain('<script>console.log("test")</script></html>')
  })

  it('should append script at end when no tags present', () => {
    const html = 'Just some text'
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toBe('Just some text<script>console.log("test")</script>')
    expect(result.endsWith('<script>console.log("test")</script>')).toBe(true)
  })

  it('should handle empty HTML', () => {
    const html = ''
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toBe('<script>console.log("test")</script>')
  })

  it('should handle HTML with multiple body tags (should inject before first)', () => {
    const html = '<html><body>First</body><body>Second</body></html>'
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toBe(
      '<html><body>First<script>console.log("test")</script></body><body>Second</body></html>',
    )
  })

  it('should handle HTML with existing script tags', () => {
    const html = '<html><body><script>existing</script>Content</body></html>'
    const result = injectScriptIntoHtml(html, testScript)

    expect(result).toContain('<script>existing</script>')
    expect(result).toContain('<script>console.log("test")</script></body>')
  })

  it('should handle complex script content', () => {
    const complexScript = `
      function init() {
        console.log('complex');
      }
      init();
    `
    const html = '<html><body>Test</body></html>'
    const result = injectScriptIntoHtml(html, complexScript)

    expect(result).toContain(complexScript)
    expect(result).toContain('</script></body>')
  })

  it('should preserve HTML case sensitivity', () => {
    const html = '<HTML><BODY>Test</BODY></HTML>'
    const result = injectScriptIntoHtml(html, testScript)

    // Should NOT match uppercase tags (our injection is case-sensitive)
    expect(result).toBe('<HTML><BODY>Test</BODY></HTML><script>console.log("test")</script>')
  })
})

describe('createModifiedHtmlHeaders', () => {
  it('should copy most headers', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'x-custom-header': 'value',
      server: 'nginx',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['content-type']).toBe('text/html')
    expect(result['x-custom-header']).toBe('value')
    expect(result.server).toBe('nginx')
  })

  it('should remove content-length header', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'content-length': '1234',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['content-length']).toBeUndefined()
  })

  it('should remove content-encoding header', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'content-encoding': 'gzip',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['content-encoding']).toBeUndefined()
  })

  it('should remove cache-related headers', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      etag: '"abc123"',
      'last-modified': 'Wed, 21 Oct 2015 07:28:00 GMT',
      'cache-control': 'public, max-age=3600',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result.etag).toBeUndefined()
    expect(result['last-modified']).toBeUndefined()
    // cache-control should be overridden, not removed
    expect(result['cache-control']).toBe('no-cache, no-store, must-revalidate')
  })

  it('should add cache-busting headers', () => {
    const originalHeaders = {
      'content-type': 'text/html',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['cache-control']).toBe('no-cache, no-store, must-revalidate')
    expect(result.pragma).toBe('no-cache')
    expect(result.expires).toBe('0')
  })

  it('should override existing cache-control header', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'cache-control': 'public, max-age=3600',
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['cache-control']).toBe('no-cache, no-store, must-revalidate')
  })

  it('should handle headers with array values', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'set-cookie': ['cookie1=value1', 'cookie2=value2'],
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['set-cookie']).toEqual(['cookie1=value1', 'cookie2=value2'])
  })

  it('should handle empty headers object', () => {
    const originalHeaders = {}

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['cache-control']).toBe('no-cache, no-store, must-revalidate')
    expect(result.pragma).toBe('no-cache')
    expect(result.expires).toBe('0')
  })

  it('should handle undefined header values', () => {
    const originalHeaders = {
      'content-type': 'text/html',
      'x-optional': undefined,
    }

    const result = createModifiedHtmlHeaders(originalHeaders)

    expect(result['x-optional']).toBeUndefined()
    expect(result['content-type']).toBe('text/html')
  })
})
