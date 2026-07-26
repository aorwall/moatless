import { useEffect, useRef, useState } from 'react'
import { HTML_PREVIEW_SANDBOX, IFRAME_ALLOW } from '@/lib/iframe-permissions'

// Static host page shipped in `public/`. The file content is *not* rendered with
// `srcDoc`: an `about:srcdoc` document inherits the embedder's CSP, and the app's
// policy (`default-src 'none'; script-src 'self'`) blocks the CDN `<script src>`
// and remote `<link rel="stylesheet">` tags workspace HTML files routinely use.
// Loading this page over HTTP gives the preview its own permissive policy.
const PREVIEW_FRAME_SRC = '/html-preview-frame.html'

interface HtmlPreviewProps {
  content: string
  filePath?: string
}

export function HtmlPreview({ content, filePath }: HtmlPreviewProps) {
  const frameRef = useRef<HTMLIFrameElement>(null)
  const [isFrameReady, setIsFrameReady] = useState(false)

  useEffect(() => {
    if (!isFrameReady) {
      return
    }
    // Targeted at `*` because the sandboxed frame has an opaque origin and so
    // matches no origin string. That is safe here: we control the frame's `src`,
    // and it only accepts messages coming from its own parent.
    frameRef.current?.contentWindow?.postMessage(
      { type: 'moatless:html-preview', html: content, allow: IFRAME_ALLOW },
      '*',
    )
  }, [content, isFrameReady])

  return (
    <iframe
      ref={frameRef}
      // Workspace files are untrusted. The sandbox lets the preview run
      // scripts/forms and use device features, but deliberately omits
      // `allow-same-origin`: the host page is served from our own origin, so
      // granting it would let the file execute code as the app itself. Without
      // that token the frame gets a unique opaque origin and cannot reach the
      // parent's cookies, storage, or DOM.
      sandbox={HTML_PREVIEW_SANDBOX}
      allow={IFRAME_ALLOW}
      src={PREVIEW_FRAME_SRC}
      onLoad={() => setIsFrameReady(true)}
      title={filePath ? `Preview of ${filePath}` : 'HTML preview'}
      className="w-full h-full border-0 bg-white"
    />
  )
}
