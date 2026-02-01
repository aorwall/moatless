import { useEffect, useState } from 'react'
import { useFileContent } from '../hooks/use-file-content'
import { cn } from '../lib/cn'
import { highlightLine } from '../utils/syntax-highlight'
import { CodeIcon, DiffIcon, EyeIcon } from './icons'
import { DiffFileView } from './diff-file-view'
import { MarkdownPreview } from './markdown-preview'
import { SyntaxToken } from './syntax-token'

type ViewMode = 'code' | 'diff' | 'preview'

function isMarkdownFile(filePath: string): boolean {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  return ext === 'md' || ext === 'mdx'
}

interface FileViewerProps {
  filePath: string | null
}

/** Detect language label from file extension for display */
function getLanguageLabel(filePath: string): string {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  const map: Record<string, string> = {
    ts: 'TypeScript',
    tsx: 'TypeScript (JSX)',
    js: 'JavaScript',
    jsx: 'JavaScript (JSX)',
    py: 'Python',
    rb: 'Ruby',
    go: 'Go',
    rs: 'Rust',
    java: 'Java',
    css: 'CSS',
    html: 'HTML',
    json: 'JSON',
    yaml: 'YAML',
    yml: 'YAML',
    md: 'Markdown',
    sh: 'Shell',
    bash: 'Shell',
    sql: 'SQL',
    toml: 'TOML',
    xml: 'XML',
    svg: 'SVG',
    dockerfile: 'Dockerfile',
  }
  return map[ext] || ext.toUpperCase() || 'Plain Text'
}

/** Get hljs-compatible language key from file extension */
function getHljsLanguage(filePath: string): string | undefined {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  const map: Record<string, string> = {
    ts: 'typescript',
    tsx: 'tsx',
    js: 'javascript',
    jsx: 'jsx',
    py: 'python',
    go: 'go',
    rs: 'rust',
    java: 'java',
    css: 'css',
    html: 'html',
    json: 'json',
    yaml: 'yaml',
    yml: 'yaml',
    md: 'markdown',
    sh: 'bash',
    bash: 'bash',
    sql: 'sql',
    xml: 'xml',
    svg: 'xml',
  }
  return map[ext]
}

interface ToggleButtonProps {
  active: boolean
  onClick: () => void
  icon: React.ReactNode
  label: string
}

function ToggleButton({ active, onClick, icon, label }: ToggleButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-1 px-2 py-1 text-xs transition-colors',
        active ? 'text-[var(--foreground)]' : 'text-[var(--muted-foreground)] hover:text-[var(--foreground)]',
      )}
      style={active ? { background: 'var(--muted)' } : undefined}
    >
      {icon}
      {label}
    </button>
  )
}

export function FileViewer({ filePath }: FileViewerProps) {
  const [viewMode, setViewMode] = useState<ViewMode>('code')
  const { data, isLoading, error } = useFileContent(filePath, true)
  const isMarkdown = filePath ? isMarkdownFile(filePath) : false
  const canDiff = data?.isModified && data?.oldContent !== undefined

  // Fall back to code mode if diff not available for current file
  useEffect(() => {
    if (viewMode === 'diff' && !canDiff) {
      setViewMode('code')
    }
    if (viewMode === 'preview' && !isMarkdown) {
      setViewMode('code')
    }
  }, [viewMode, canDiff, isMarkdown])

  if (!filePath) {
    return (
      <div className="h-full flex items-center justify-center" style={{ color: 'var(--muted-foreground)' }}>
        <div className="text-center space-y-2">
          <div className="text-4xl opacity-30">{ '{ }' }</div>
          <p className="text-sm">Select a file to view its contents</p>
        </div>
      </div>
    )
  }

  if (isLoading) {
    return (
      <div className="h-full flex flex-col">
        <div className="p-3 flex-shrink-0" style={{ borderBottom: '1px solid var(--border)' }}>
          <div className="h-4 w-48 rounded animate-pulse" style={{ background: 'var(--muted)' }} />
        </div>
        <div className="flex-1 p-4 space-y-2">
          {Array.from({ length: 20 }).map((_, i) => (
            <div key={i} className="h-4 rounded animate-pulse" style={{ background: 'var(--muted)', width: `${30 + Math.random() * 60}%` }} />
          ))}
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-center space-y-2" style={{ color: 'var(--destructive)' }}>
          <p className="text-sm font-medium">Failed to load file</p>
          <p className="text-xs">{error.message}</p>
        </div>
      </div>
    )
  }

  if (!data) return null

  const languageLabel = getLanguageLabel(filePath)
  const hljsLang = getHljsLanguage(filePath)
  const lines = data.content.split('\n')
  const showToggle = canDiff || isMarkdown

  return (
    <div className="h-full flex flex-col">
      {/* File header */}
      <div className="p-3 flex-shrink-0 flex items-center justify-between" style={{ borderBottom: '1px solid var(--border)' }}>
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-mono truncate">{filePath}</span>
          <span className="text-xs px-1.5 py-0.5 rounded shrink-0" style={{ background: 'var(--muted)', color: 'var(--muted-foreground)' }}>
            {languageLabel}
          </span>
        </div>
        <div className="flex items-center gap-2 shrink-0 ml-2">
          {data.isModified && (
            <div className="flex items-center gap-2 text-xs">
              <span style={{ color: 'var(--git-added)' }}>+{data.additions}</span>
              <span style={{ color: 'var(--git-deleted)' }}>&minus;{data.deletions}</span>
            </div>
          )}
          {showToggle && (
            <div className="flex items-center rounded-md overflow-hidden" style={{ border: '1px solid var(--border)' }}>
              <ToggleButton
                active={viewMode === 'code'}
                onClick={() => setViewMode('code')}
                icon={<CodeIcon className="w-3.5 h-3.5" />}
                label="Code"
              />
              {canDiff && (
                <ToggleButton
                  active={viewMode === 'diff'}
                  onClick={() => setViewMode('diff')}
                  icon={<DiffIcon className="w-3.5 h-3.5" />}
                  label="Diff"
                />
              )}
              {isMarkdown && (
                <ToggleButton
                  active={viewMode === 'preview'}
                  onClick={() => setViewMode('preview')}
                  icon={<EyeIcon className="w-3.5 h-3.5" />}
                  label="Preview"
                />
              )}
            </div>
          )}
        </div>
      </div>

      {/* File content */}
      <div className="flex-1 overflow-auto">
        {viewMode === 'preview' && isMarkdown ? (
          <MarkdownPreview content={data.content} />
        ) : viewMode === 'diff' && canDiff ? (
          <DiffFileView
            oldContent={data.oldContent ?? ''}
            newContent={data.content}
            filename={filePath}
          />
        ) : (
          <pre className="text-sm leading-relaxed">
            <code>
              {lines.map((line, i) => (
                <div key={i} className="flex hover:bg-[var(--accent)] transition-colors">
                  <span
                    className="select-none text-right pr-4 pl-4 shrink-0"
                    style={{ color: 'var(--muted-foreground)', width: '4rem', minWidth: '4rem' }}
                  >
                    {i + 1}
                  </span>
                  <span className="pr-4 whitespace-pre">
                    {hljsLang && line ? (
                      highlightLine(line, hljsLang).map((token, j) => (
                        <SyntaxToken key={`${token.type}-${j}`} token={token} />
                      ))
                    ) : (
                      line || ' '
                    )}
                  </span>
                </div>
              ))}
            </code>
          </pre>
        )}
      </div>
    </div>
  )
}
