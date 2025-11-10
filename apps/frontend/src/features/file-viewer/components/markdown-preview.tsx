import type mermaidModule from 'mermaid'
import type React from 'react'
import { useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useTheme } from '@/lib/components/theme-provider'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/lib/components/ui/dialog'
import { cn } from '@/lib/utils'

// Lazy load mermaid only when diagrams are present
let mermaidPromise: Promise<typeof mermaidModule> | null = null
const loadMermaid = () => {
  if (!mermaidPromise) {
    mermaidPromise = import('mermaid').then((mod) => mod.default)
  }
  return mermaidPromise
}

interface MarkdownPreviewProps {
  content: string
  className?: string
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { theme } = useTheme()
  const renderingRef = useRef<Set<string>>(new Set())
  const [expandedDiagram, setExpandedDiagram] = useState<{
    svg: string
    definition: string
  } | null>(null)

  useEffect(() => {
    const isDark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)

    // Load and initialize mermaid lazily when theme changes
    loadMermaid()
      .then((mermaid) => {
        mermaid.initialize({
          theme: isDark ? 'dark' : 'default',
          startOnLoad: false,
          fontFamily: 'ui-monospace, monospace',
          securityLevel: 'strict',
          logLevel: 'error',
        })
      })
      .catch((_error) => {
        // Silently fail - mermaid initialization errors are non-critical
      })
  }, [theme])

  useEffect(() => {
    // Clear previous rendering set when content changes
    renderingRef.current.clear()

    if (containerRef.current) {
      const mermaidElements = containerRef.current.querySelectorAll('.language-mermaid')
      const promises: Promise<void>[] = []

      for (const [index, element] of Array.from(mermaidElements).entries()) {
        const graphDefinition = element.textContent || ''
        const elementId = `mermaid-${index}`

        // Skip if already rendering
        if (renderingRef.current.has(elementId)) {
          return
        }

        renderingRef.current.add(elementId)
        const graphId = `mermaid-${Date.now()}-${index}-${Math.random().toString(36).substr(2, 9)}`

        const promise = loadMermaid()
          .then((mermaid) => mermaid.render(graphId, graphDefinition))
          .then(({ svg }) => {
            // Only update if element is still in DOM
            if (element.parentNode) {
              // Create a wrapper div with the SVG and an expand button
              const wrapper = document.createElement('div')
              wrapper.className = 'relative group cursor-pointer transition-all hover:opacity-90'
              wrapper.innerHTML = `
                  ${svg}
                  <div class="absolute inset-0 bg-foreground/5 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none"></div>
                  <button
                    class="absolute top-2 right-2 p-2 bg-background/90 rounded-md opacity-0 group-hover:opacity-100 transition-opacity shadow-sm border pointer-events-auto"
                    aria-label="Expand diagram"
                  >
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="m21 21-6-6m6 6v-4.8m0 4.8h-4.8"/>
                      <path d="M3 16.2V21m0 0h4.8M3 21l6-6"/>
                      <path d="M21 7.8V3m0 0h-4.8M21 3l-6 6"/>
                      <path d="M3 7.8V3m0 0h4.8M3 3l6 6"/>
                    </svg>
                  </button>
                `

              // Add click handler to the wrapper
              wrapper.addEventListener('click', (e) => {
                e.preventDefault()
                setExpandedDiagram({ svg, definition: graphDefinition })
              })

              // Replace the pre element content with the wrapper
              element.innerHTML = ''
              element.appendChild(wrapper)
              element.classList.remove('language-mermaid')
              element.classList.add('mermaid-rendered')
            }
          })
          .catch((error) => {
            // Only update if element is still in DOM
            if (element.parentNode) {
              try {
                // Create a more user-friendly error display
                const errorWrapper = document.createElement('div')
                errorWrapper.className =
                  'border border-destructive/20 rounded-lg p-4 bg-destructive/5 my-4'
                errorWrapper.innerHTML = `
                  <div class="flex items-start gap-3">
                    <div class="flex-shrink-0 mt-0.5">
                      <svg class="w-5 h-5 text-destructive" viewBox="0 0 20 20" fill="currentColor">
                        <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clip-rule="evenodd" />
                      </svg>
                    </div>
                    <div class="flex-1 min-w-0">
                      <h4 class="text-sm font-medium text-destructive mb-1">Mermaid Diagram Error</h4>
                      <p class="text-sm text-muted-foreground mb-2">${error.message}</p>
                      <details class="mt-2">
                        <summary class="text-xs text-muted-foreground cursor-pointer hover:text-foreground">
                          Show diagram source
                        </summary>
                        <pre class="mt-2 text-xs bg-muted/50 p-2 rounded border overflow-x-auto"><code>${graphDefinition}</code></pre>
                      </details>
                    </div>
                  </div>
                `

                // Replace the pre element content with the error wrapper
                element.innerHTML = ''
                element.appendChild(errorWrapper)
                element.classList.remove('language-mermaid')
                element.classList.add('mermaid-error')
              } catch (_domError) {
                element.innerHTML = `<div class="text-destructive p-2 border border-destructive/20 rounded">Error rendering diagram: ${error.message}</div>`
                element.classList.remove('language-mermaid')
                element.classList.add('mermaid-error')
              }
            }
          })
          .finally(() => {
            renderingRef.current.delete(elementId)
          })

        promises.push(promise)
      }

      Promise.all(promises).catch((_error) => {
        // Silently fail - mermaid rendering errors are shown inline
      })
    }

    // Cleanup function
    return () => {
      renderingRef.current.clear()
    }
  }, [])

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
              <pre className="overflow-x-auto rounded-md bg-muted p-4 text-sm" {...props}>
                {children}
              </pre>
            )
          },
          code: ({ children, className, ...props }) => {
            const isInline = !className
            return isInline ? (
              <code className="px-1 py-0.5 rounded bg-muted text-foreground text-sm" {...props}>
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
            <img alt={alt} src={src} className="max-w-full h-auto" {...props} />
          ),
        }}
      >
        {content}
      </ReactMarkdown>

      {/* Expanded Diagram Dialog */}
      <Dialog open={!!expandedDiagram} onOpenChange={(open) => !open && setExpandedDiagram(null)}>
        <DialogContent className="max-w-[90vw] max-h-[90vh] overflow-auto">
          <DialogHeader>
            <DialogTitle>Mermaid Diagram</DialogTitle>
          </DialogHeader>
          <div className="flex justify-center items-center p-4">
            {expandedDiagram && (
              <div
                // biome-ignore lint/security/noDangerouslySetInnerHtml: SVG content from Mermaid is safe
                dangerouslySetInnerHTML={{ __html: expandedDiagram.svg }}
                className="[&>svg]:max-w-full [&>svg]:h-auto"
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
