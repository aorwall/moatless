import { useEffect, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import mermaid from 'mermaid'
import { cn } from '@/lib/utils'
import { useTheme } from '@/lib/components/theme-provider'

interface MarkdownPreviewProps {
  content: string
  className?: string
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { theme } = useTheme()

  useEffect(() => {
    const isDark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)

    mermaid.initialize({
      theme: isDark ? 'dark' : 'default',
      startOnLoad: false,
      fontFamily: 'ui-monospace, monospace',
      securityLevel: 'strict',
      logLevel: 'error',
    })
  }, [theme])

  useEffect(() => {
    if (containerRef.current) {
      const mermaidElements = containerRef.current.querySelectorAll('.language-mermaid')
      const promises: Promise<void>[] = []

      mermaidElements.forEach((element, index) => {
        const graphDefinition = element.textContent || ''
        const graphId = `mermaid-${Date.now()}-${index}`

        const promise = mermaid
          .render(graphId, graphDefinition)
          .then(({ svg }) => {
            element.innerHTML = svg
            element.classList.remove('language-mermaid')
            element.classList.add('mermaid-rendered')
          })
          .catch((error) => {
            console.error('Mermaid rendering error:', error)
            element.innerHTML = `<pre class="text-destructive">Error rendering diagram: ${error.message}</pre>`
          })

        promises.push(promise)
      })

      Promise.all(promises).catch(console.error)
    }
  }, [content])

  return (
    <div
      ref={containerRef}
      className={cn(
        'prose prose-sm dark:prose-invert max-w-full p-6 min-w-0',
        'prose-p:my-2',
        'prose-pre:bg-muted',
        'prose-pre:text-foreground',
        'prose-code:bg-muted',
        'prose-code:text-foreground',
        'prose-code:px-1 prose-code:py-0.5 prose-code:rounded',
        'prose-code:before:content-none prose-code:after:content-none',
        'prose-strong:text-current',
        'prose-a:text-primary',
        'prose-a:no-underline prose-a:hover:underline',
        'prose-ul:my-2 prose-ol:my-2',
        'prose-li:my-0',
        'prose-headings:scroll-mt-20',
        'prose-table:overflow-x-auto prose-table:block prose-table:whitespace-nowrap',
        className,
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children, ...props }) => {
            const codeElement = children as React.ReactElement
            const codeElementProps = codeElement?.props as { className?: string } | undefined
            const className = codeElementProps?.className || ''
            const isMermaid = className.includes('language-mermaid')

            if (isMermaid) {
              return (
                <div className="my-4 flex justify-center overflow-x-auto max-w-full">
                  <pre className={className} {...props}>
                    {children}
                  </pre>
                </div>
              )
            }

            return (
              <pre
                className="overflow-x-auto rounded-md bg-muted p-4 text-sm"
                {...props}
              >
                {children}
              </pre>
            )
          },
          code: ({ children, className, ...props }) => {
            const isInline = !className
            return isInline ? (
              <code
                className="px-1 py-0.5 rounded bg-muted text-foreground text-sm"
                {...props}
              >
                {children}
              </code>
            ) : (
              <code className={className} {...props}>
                {children}
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
          img: ({ alt, src, ...props }) => (
            <img
              alt={alt}
              src={src}
              className="max-w-full h-auto"
              {...props}
            />
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  )
}