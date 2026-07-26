import { readFileSync } from 'node:fs'
import path from 'node:path'
import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { IFRAME_ALLOW } from '@/lib/iframe-permissions'
import { HtmlPreview } from '../html-preview'

// The previewed file is handed to a host page rather than rendered with
// `srcDoc`, because an `about:srcdoc` document inherits the app's
// `script-src 'self'` CSP and would block the CDN `<script>`/`<link>` tags
// workspace HTML files routinely use. These tests pin the two halves of that
// arrangement together: the component's postMessage payload and the listener in
// `public/html-preview-frame.html`, which live in different files and would
// otherwise drift silently.
const HOST_PAGE_SRC = '/html-preview-frame.html'
const MESSAGE_TYPE = 'moatless:html-preview'

function renderPreview(content: string) {
  const postMessage = vi.fn()
  const view = render(<HtmlPreview content={content} filePath="index.html" />)
  const frame = screen.getByTitle('Preview of index.html') as HTMLIFrameElement

  // jsdom never navigates the frame, so stand in for the loaded host page.
  Object.defineProperty(frame, 'contentWindow', { value: { postMessage }, writable: true })

  return { frame, postMessage, view }
}

describe('HtmlPreview', () => {
  it('points the frame at the host page instead of inlining the file with srcDoc', () => {
    const { frame } = renderPreview('<h1>hi</h1>')

    expect(frame.getAttribute('src')).toBe(HOST_PAGE_SRC)
    expect(frame.getAttribute('srcdoc')).toBeNull()
    // Untrusted workspace content must never run as the app itself.
    expect(frame.getAttribute('sandbox')).not.toContain('allow-same-origin')
  })

  it('sends the file to the host page once the frame has loaded', () => {
    const content = '<script src="https://cdn.tailwindcss.com"></script><p>hi</p>'
    const { frame, postMessage } = renderPreview(content)

    expect(postMessage).not.toHaveBeenCalled()

    act(() => {
      frame.dispatchEvent(new Event('load'))
    })

    expect(postMessage).toHaveBeenCalledWith(
      { type: MESSAGE_TYPE, html: content, allow: IFRAME_ALLOW },
      '*',
    )
  })

  it('pushes updated content to an already-loaded frame', () => {
    const postMessage = vi.fn()
    const { rerender } = render(<HtmlPreview content="<p>one</p>" filePath="index.html" />)
    const frame = screen.getByTitle('Preview of index.html') as HTMLIFrameElement
    Object.defineProperty(frame, 'contentWindow', { value: { postMessage }, writable: true })

    act(() => {
      frame.dispatchEvent(new Event('load'))
    })
    rerender(<HtmlPreview content="<p>two</p>" filePath="index.html" />)

    expect(postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({ html: '<p>two</p>' }),
      '*',
    )
  })

  it('matches the contract the host page listens for', () => {
    const hostPage = readFileSync(
      path.resolve(__dirname, '../../../../../public/html-preview-frame.html'),
      'utf8',
    )

    expect(hostPage).toContain(MESSAGE_TYPE)
    // The listener reads these fields off the message and rejects anything that
    // did not come from the embedding app.
    expect(hostPage).toContain('message.html')
    expect(hostPage).toContain('message.allow')
    expect(hostPage).toContain('event.source !== window.parent')
  })
})
