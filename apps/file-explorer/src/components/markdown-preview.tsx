import type React from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { highlightLine } from '../utils/syntax-highlight'
import { SyntaxToken } from './syntax-token'

interface MarkdownPreviewProps {
  content: string
}

/** Extract language from className like "language-typescript" */
function extractLanguage(className: string | undefined): string | undefined {
  if (!className) return undefined
  const match = className.match(/language-(\w+)/)
  return match?.[1]
}

/** Render syntax-highlighted code block content */
function HighlightedCode({ code, language }: { code: string; language?: string }) {
  const lines = code.replace(/\n$/, '').split('\n')
  return (
    <>
      {lines.map((line, i) => (
        <span key={i}>
          {language && line ? (
            highlightLine(line, language).map((token, j) => (
              <SyntaxToken key={`${token.type}-${j}`} token={token} />
            ))
          ) : (
            line
          )}
          {i < lines.length - 1 ? '\n' : ''}
        </span>
      ))}
    </>
  )
}

export function MarkdownPreview({ content }: MarkdownPreviewProps) {
  return (
    <div
      className={[
        'prose prose-sm prose-invert max-w-full p-6 min-w-0',
        'prose-p:my-2',
        'prose-pre:bg-[var(--muted)]',
        'prose-pre:text-[var(--foreground)]',
        'prose-code:bg-[var(--muted)]',
        'prose-code:text-[var(--foreground)]',
        'prose-code:px-1 prose-code:py-0.5 prose-code:rounded',
        'prose-code:before:content-none prose-code:after:content-none',
        'prose-strong:text-current',
        'prose-a:text-[var(--primary)]',
        'prose-a:no-underline hover:prose-a:underline',
        'prose-ul:my-2 prose-ol:my-2',
        'prose-li:my-0',
        'prose-headings:scroll-mt-20',
        'prose-table:overflow-x-auto prose-table:block prose-table:whitespace-nowrap',
      ].join(' ')}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children, ...props }) => (
            <pre className="overflow-x-auto rounded-md p-4 text-sm" style={{ background: 'var(--muted)' }} {...props}>
              {children}
            </pre>
          ),
          code: ({ children, className, ...props }) => {
            const isInline = !className
            if (isInline) {
              return (
                <code className="px-1 py-0.5 rounded text-sm" style={{ background: 'var(--muted)', color: 'var(--foreground)' }} {...props}>
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
          img: ({ alt, src, ...props }: React.ImgHTMLAttributes<HTMLImageElement>) => (
            <img alt={alt} src={src} className="max-w-full h-auto" {...props} />
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  )
}
