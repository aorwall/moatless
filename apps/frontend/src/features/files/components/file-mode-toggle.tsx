import { Code, Eye, GitCompareArrows } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { FileViewMode } from './file-viewer'

/** One icon button in the code/diff/preview segmented control. */
function ModeTab({
  active,
  onClick,
  icon: Icon,
  label,
}: {
  active: boolean
  onClick: () => void
  icon: typeof Code
  label: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        'flex items-center justify-center px-2 py-1.5 transition-colors',
        active ? 'text-foreground bg-muted' : 'text-muted-foreground hover:text-foreground',
      )}
    >
      <Icon className="w-4 h-4" />
    </button>
  )
}

interface FileModeToggleGroupProps {
  viewMode: FileViewMode
  onViewModeChange: (viewMode: FileViewMode) => void
  canDiff: boolean
  canPreview: boolean
}

/**
 * The code / diff / preview segmented control. Shared between the file viewer's
 * header and the file tree's header (both surface it on mobile, where the
 * desktop workspace toolbar is hidden). Diff and preview tabs only appear when
 * the current file actually supports them.
 */
export function FileModeToggleGroup({
  viewMode,
  onViewModeChange,
  canDiff,
  canPreview,
}: FileModeToggleGroupProps) {
  return (
    <div className="flex items-center rounded-md overflow-hidden border flex-shrink-0">
      <ModeTab
        active={viewMode === 'code'}
        onClick={() => onViewModeChange('code')}
        icon={Code}
        label="Code"
      />
      {canDiff && (
        <ModeTab
          active={viewMode === 'diff'}
          onClick={() => onViewModeChange('diff')}
          icon={GitCompareArrows}
          label="Diff"
        />
      )}
      {canPreview && (
        <ModeTab
          active={viewMode === 'preview'}
          onClick={() => onViewModeChange('preview')}
          icon={Eye}
          label="Preview"
        />
      )}
    </div>
  )
}
