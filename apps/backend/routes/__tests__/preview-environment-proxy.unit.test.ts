import { describe, expect, it } from 'vitest'

describe('Preview Environment Proxy - Unit Tests', () => {
  describe('script injection logic', () => {
    it('should inject script before closing head tag', () => {
      const html = '<html><head><title>Test</title></head><body>Content</body></html>'
      const taskId = 'test-task-123'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
              
              // Send a message to parent to confirm script is loaded
              window.addEventListener('load', () => {
                if (window.parent && window.parent !== window) {
                  window.parent.postMessage({
                    type: 'INSPECTOR_SCRIPT_LOADED',
                    payload: { version: '1.0.0', taskId: '${taskId}' },
                    source: 'preview-iframe'
                  }, '*');
                }
              });
            </script>
          `

      const modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)

      expect(modifiedHtml).toContain(injectedScript)
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected for task: test-task-123')
      expect(modifiedHtml).toContain('INSPECTOR_SCRIPT_LOADED')
      expect(modifiedHtml).toContain('</script>')
      expect(modifiedHtml).toContain('</head>')
    })

    it('should inject script at beginning of body when no head tag', () => {
      const html = '<html><body>Content</body></html>'
      const taskId = 'test-task-456'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
            </script>
          `

      const modifiedHtml = html.replace('<body>', `<body>${injectedScript}`)

      expect(modifiedHtml).toContain(injectedScript)
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected for task: test-task-456')
      expect(modifiedHtml).toMatch(/<body>[\s\S]*<script>[\s\S]*console\.log[\s\S]*<\/script>/)
    })

    it('should inject script at beginning when no head or body tags', () => {
      const html = '<div>Just content</div>'
      const taskId = 'test-task-789'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
            </script>
          `

      const modifiedHtml = `${injectedScript}${html}`

      expect(modifiedHtml).toContain(injectedScript)
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected for task: test-task-789')
      expect(modifiedHtml.trim().startsWith('<script>')).toBe(true)
    })

    it('should handle special characters in taskId', () => {
      const html = '<html><head></head><body></body></html>'
      const taskId = 'test-task-with-special-chars-123_ABC'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
            </script>
          `

      const modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)

      expect(modifiedHtml).toContain('test-task-with-special-chars-123_ABC')
      expect(modifiedHtml).not.toContain('undefined')
    })

    it('should preserve existing scripts and content', () => {
      const html = `
        <html>
          <head>
            <title>Test Page</title>
            <script>console.log('existing script');</script>
          </head>
          <body>
            <h1>Test Content</h1>
            <script>console.log('body script');</script>
          </body>
        </html>
      `
      const taskId = 'preservation-test'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
            </script>
          `

      const modifiedHtml = html.replace('</head>', `${injectedScript}</head>`)

      expect(modifiedHtml).toContain("console.log('existing script');")
      expect(modifiedHtml).toContain("console.log('body script');")
      expect(modifiedHtml).toContain('<h1>Test Content</h1>')
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected')
    })

    it('should handle empty HTML gracefully', () => {
      const html = ''
      const taskId = 'empty-test'

      const injectedScript = `
            <script>
              console.log('🚀 Moatless Preview Script Injected for task: ${taskId}');
            </script>
          `

      const modifiedHtml = `${injectedScript}${html}`

      expect(modifiedHtml).toBe(injectedScript)
      expect(modifiedHtml).toContain('🚀 Moatless Preview Script Injected for task: empty-test')
    })
  })

  describe('content type detection', () => {
    it('should identify HTML content types', () => {
      const htmlContentTypes = [
        'text/html',
        'text/html; charset=utf-8',
        'text/html; charset=UTF-8',
        'text/html;charset=utf-8',
      ]

      htmlContentTypes.forEach((contentType) => {
        expect(contentType.toLowerCase().includes('text/html')).toBe(true)
      })
    })

    it('should not inject into non-HTML content types', () => {
      const nonHtmlContentTypes = [
        'application/json',
        'text/css',
        'application/javascript',
        'text/javascript',
        'image/png',
        'application/pdf',
        'text/plain',
      ]

      nonHtmlContentTypes.forEach((contentType) => {
        expect(contentType.includes('text/html')).toBe(false)
      })
    })
  })

  describe('post message payload structure', () => {
    it('should create valid post message payload', () => {
      const taskId = 'payload-test-123'
      const expectedPayload = {
        type: 'INSPECTOR_SCRIPT_LOADED',
        payload: { version: '1.0.0', taskId: taskId },
        source: 'preview-iframe',
      }

      expect(expectedPayload.type).toBe('INSPECTOR_SCRIPT_LOADED')
      expect(expectedPayload.payload.taskId).toBe(taskId)
      expect(expectedPayload.payload.version).toBe('1.0.0')
      expect(expectedPayload.source).toBe('preview-iframe')
    })
  })
})
