interface HtmlPreviewProps {
  content: string
  filePath?: string
}

export function HtmlPreview({ content, filePath }: HtmlPreviewProps) {
  return (
    <iframe
      // Fully sandboxed: an empty `sandbox` applies every restriction, so the
      // preview renders HTML/CSS but cannot run scripts or submit forms.
      // Workspace files are untrusted, so previewing one must never execute
      // code in the user's browser. (Passive subresources such as external
      // <img>/<link> are still fetched by the browser; the sandbox does not
      // block those.)
      sandbox=""
      srcDoc={content}
      title={filePath ? `Preview of ${filePath}` : 'HTML preview'}
      className="w-full h-full border-0 bg-white"
    />
  )
}
