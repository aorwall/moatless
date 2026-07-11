import { type HighlightToken, highlightLine, SyntaxToken } from '@moatless/diff-viewer'
import type React from 'react'
import { useEffect, useMemo, useState } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { fetchFileContent } from '../api/files-api'

interface MarkdownPreviewProps {
  content: string
  taskId?: string
  filePath?: string
}

function extractLanguage(className: string | undefined): string | undefined {
  if (!className) {
    return
  }
  const match = className.match(/language-(\w+)/)
  return match?.[1]
}

function HighlightedCode({ code, language }: { code: string; language?: string | undefined }) {
  const lines = code.replace(/\n$/, '').split('\n')
  return (
    <>
      {lines.map((line, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: line number is the natural key
        <span key={i}>
          {language && line
            ? highlightLine(line, language).map((token: HighlightToken, j: number) => (
                <SyntaxToken key={`${token.type}-${j}`} token={token} />
              ))
            : line}
          {i < lines.length - 1 ? '\n' : ''}
        </span>
      ))}
    </>
  )
}

function isRelativePath(src: string | undefined): boolean {
  if (!src) {
    return false
  }
  return !(src.startsWith('http://') || src.startsWith('https://') || src.startsWith('data:'))
}

function resolveRelativePath(src: string, markdownFilePath: string): string {
  const dir = markdownFilePath.includes('/')
    ? markdownFilePath.substring(0, markdownFilePath.lastIndexOf('/'))
    : ''
  const cleanSrc = src.startsWith('./') ? src.slice(2) : src
  return dir ? `${dir}/${cleanSrc}` : cleanSrc
}

function MarkdownImage({
  src,
  alt,
  taskId,
  markdownFilePath,
  ...props
}: React.ImgHTMLAttributes<HTMLImageElement> & {
  taskId?: string | undefined
  markdownFilePath?: string | undefined
}) {
  const [resolvedSrc, setResolvedSrc] = useState(src)
  const [error, setError] = useState(false)

  useEffect(() => {
    if (!(taskId && markdownFilePath && src && isRelativePath(src))) {
      return
    }

    const imagePath = resolveRelativePath(src, markdownFilePath)
    let cancelled = false

    fetchFileContent(taskId, imagePath, false)
      .then((response) => {
        if (cancelled) {
          return
        }
        if (response.isBinary && response.mediaType) {
          setResolvedSrc(`data:${response.mediaType};base64,${response.content}`)
        }
      })
      .catch(() => {
        if (!cancelled) {
          setError(true)
        }
      })

    return () => {
      cancelled = true
    }
  }, [taskId, markdownFilePath, src])

  if (error) {
    return (
      <span className="text-muted-foreground text-sm italic">[Image not found: {alt || src}]</span>
    )
  }

  return <img alt={alt} src={resolvedSrc} className="max-w-full h-auto" {...props} />
}

// Static renderers close over nothing that changes per render, so they live at
// module scope. Only the `img` renderer (which needs `taskId`/`filePath`) is
// added per-instance below.
const STATIC_MARKDOWN_COMPONENTS = {
  pre: ({ children, ...props }) => (
    <pre className="overflow-x-auto rounded-md p-4 text-sm bg-muted" {...props}>
      {children}
    </pre>
  ),
  code: ({ children, className, ...props }) => {
    const isInline = !className
    if (isInline) {
      return (
        <code className="px-1 py-0.5 rounded text-sm bg-muted text-foreground" {...props}>
          {children}
        </code>
      )
    }

    const language = extractLanguage(className)
    const codeString = typeof children === 'string' ? children : String(children ?? '')

    return (
      <code className={className} {...props}>
        <HighlightedCode code={codeString} language={language} />
      </code>
    )
  },
  table: ({ children, ...props }) => (
    <div className="overflow-x-auto max-w-full">
      <table className="min-w-full" {...props}>
        {children}
      </table>
    </div>
  ),
} satisfies Components

const REMARK_PLUGINS = [remarkGfm]

export function MarkdownPreview({ content, taskId, filePath }: MarkdownPreviewProps) {
  // Memoize so react-markdown sees a stable `components` identity across parent
  // re-renders; it's only rebuilt when the image-resolution inputs change.
  const components = useMemo<Components>(
    () => ({
      ...STATIC_MARKDOWN_COMPONENTS,
      img: ({ alt, src, ...props }: React.ImgHTMLAttributes<HTMLImageElement>) => (
        <MarkdownImage alt={alt} src={src} taskId={taskId} markdownFilePath={filePath} {...props} />
      ),
    }),
    [taskId, filePath],
  )

  return (
    <div className="prose prose-sm dark:prose-invert max-w-full p-6 min-w-0 prose-p:my-2 prose-pre:bg-muted prose-pre:text-foreground prose-code:bg-muted prose-code:text-foreground prose-code:px-1 prose-code:py-0.5 prose-code:rounded prose-code:before:content-none prose-code:after:content-none prose-strong:text-current prose-a:text-primary prose-a:no-underline hover:prose-a:underline prose-ul:my-2 prose-ol:my-2 prose-li:my-0 prose-headings:scroll-mt-20 prose-table:overflow-x-auto prose-table:block prose-table:whitespace-nowrap">
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  )
}
