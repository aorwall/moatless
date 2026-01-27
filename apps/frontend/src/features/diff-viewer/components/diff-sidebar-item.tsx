import { type DiffFile, FileChangeStats } from '@moatless/diff-viewer'
import { FileCode, FileMinus, FilePlus, FileText } from 'lucide-react'
import { cn } from '@/lib/utils'

interface DiffSidebarItemProps {
  file: DiffFile
  isSelected: boolean
  onClick: () => void
}

/**
 * Renders a single file item in the diff sidebar
 */
export function DiffSidebarItem({ file, isSelected, onClick }: DiffSidebarItemProps) {
  const getStatusIcon = () => {
    switch (file.status) {
      case 'added':
        return <FilePlus className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
      case 'deleted':
        return <FileMinus className="h-4 w-4 text-red-600 dark:text-red-400 shrink-0" />
      case 'renamed':
        return <FileCode className="h-4 w-4 text-blue-600 dark:text-blue-400 shrink-0" />
      default:
        return <FileText className="h-4 w-4 text-amber-600 dark:text-amber-400 shrink-0" />
    }
  }

  // Get just the filename from the path
  const parts = file.filename.split('/')
  const filename = parts[parts.length - 1] ?? file.filename
  const directory = parts.length > 1 ? parts.slice(0, -1).join('/') : undefined

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'w-full flex items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted/50 transition-colors',
        isSelected && 'bg-muted',
      )}
    >
      {getStatusIcon()}
      <div className="min-w-0 flex-1 truncate">
        {directory && <span className="text-muted-foreground text-xs">{directory}/</span>}
        <span className="font-medium">{filename}</span>
      </div>
      <FileChangeStats additions={file.additions} deletions={file.deletions} size="sm" />
    </button>
  )
}
