import { useFileContent } from '../hooks/use-file-content'
import { cn } from '../lib/cn'

interface FileViewerProps {
  filePath: string | null
}

/** Detect language from file extension for display */
function getLanguage(filePath: string): string {
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

export function FileViewer({ filePath }: FileViewerProps) {
  const { data, isLoading, error } = useFileContent(filePath, true)

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

  const language = getLanguage(filePath)
  const lines = data.content.split('\n')

  return (
    <div className="h-full flex flex-col">
      {/* File header */}
      <div className="p-3 flex-shrink-0 flex items-center justify-between" style={{ borderBottom: '1px solid var(--border)' }}>
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-mono truncate">{filePath}</span>
          <span className="text-xs px-1.5 py-0.5 rounded shrink-0" style={{ background: 'var(--muted)', color: 'var(--muted-foreground)' }}>
            {language}
          </span>
        </div>
        {data.isModified && (
          <div className="flex items-center gap-2 text-xs shrink-0 ml-2">
            <span style={{ color: 'var(--git-added)' }}>+{data.additions}</span>
            <span style={{ color: 'var(--git-deleted)' }}>&minus;{data.deletions}</span>
          </div>
        )}
      </div>

      {/* File content */}
      <div className="flex-1 overflow-auto">
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
                <span className="pr-4 whitespace-pre">{line || ' '}</span>
              </div>
            ))}
          </code>
        </pre>
      </div>
    </div>
  )
}
