import { HTML_PREVIEW_SANDBOX, IFRAME_ALLOW } from '@/lib/iframe-permissions'

interface HtmlPreviewProps {
  content: string
  filePath?: string
}

export function HtmlPreview({ content, filePath }: HtmlPreviewProps) {
  return (
    <iframe
      // Workspace files are untrusted. The sandbox lets the preview run
      // scripts/forms and use device features, but deliberately omits
      // `allow-same-origin`: a `srcDoc` frame inherits our origin, so granting
      // it would let the file execute code as the app itself. Without that
      // token the frame gets a unique opaque origin and cannot reach the
      // parent's cookies, storage, or DOM.
      sandbox={HTML_PREVIEW_SANDBOX}
      allow={IFRAME_ALLOW}
      srcDoc={content}
      title={filePath ? `Preview of ${filePath}` : 'HTML preview'}
      className="w-full h-full border-0 bg-white"
    />
  )
}
