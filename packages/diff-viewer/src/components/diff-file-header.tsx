import type { DiffFileHeaderProps } from '../types/index.js'
import { FileChangeStats } from './file-change-stats.js'

/**
 * Renders the header for a diff file card
 * Includes collapse button, file path, stats, and optional actions
 */
export function DiffFileHeader({ file, collapsed = false, onCollapseChange }: DiffFileHeaderProps) {
  const getStatusIcon = () => {
    switch (file.status) {
      case 'added':
        return (
          <span className="flex h-5 w-5 items-center justify-center rounded text-xs font-medium text-green-700 dark:text-green-400">
            A
          </span>
        )
      case 'deleted':
        return (
          <span className="flex h-5 w-5 items-center justify-center rounded text-xs font-medium text-red-700 dark:text-red-400">
            D
          </span>
        )
      case 'renamed':
        return (
          <span className="flex h-5 w-5 items-center justify-center rounded text-xs font-medium text-blue-700 dark:text-blue-400">
            R
          </span>
        )
      default:
        return (
          <span className="flex h-5 w-5 items-center justify-center rounded text-xs font-medium text-amber-700 dark:text-amber-400">
            M
          </span>
        )
    }
  }

  const formatFilePath = () => {
    const parts = file.filename.split('/')
    if (parts.length <= 1) {
      return <span className="font-medium">{file.filename}</span>
    }

    const dir = parts.slice(0, -1).join('/')
    const name = parts.at(-1)

    return (
      <>
        <span className="text-muted-foreground">{dir}/</span>
        <span className="font-medium">{name}</span>
      </>
    )
  }

  return (
    <div className="flex items-center gap-2 border-b border-border bg-muted/30 px-3 py-2">
      {/* Collapse button */}
      <button
        type="button"
        onClick={() => onCollapseChange?.(!collapsed)}
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded hover:bg-muted"
        aria-label={collapsed ? 'Expand file' : 'Collapse file'}
      >
        <svg
          className={`h-4 w-4 text-muted-foreground transition-transform ${collapsed ? '' : 'rotate-90'}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          aria-hidden="true"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
      </button>

      {/* Status indicator */}
      {getStatusIcon()}

      {/* File path */}
      <div className="min-w-0 flex-1 truncate font-mono text-sm">
        {file.oldFilename && file.status === 'renamed' && (
          <>
            <span className="text-muted-foreground line-through">{file.oldFilename}</span>
            <span className="mx-2 text-muted-foreground">→</span>
          </>
        )}
        {formatFilePath()}
      </div>

      {/* Language badge */}
      {file.language && (
        <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
          {file.language}
        </span>
      )}

      {/* Stats */}
      <FileChangeStats additions={file.additions} deletions={file.deletions} size="sm" />
    </div>
  )
}
