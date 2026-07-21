import { Code, ExternalLink, Eye, GitCompareArrows } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { useFileContent } from '../hooks/use-file-content'
import { useFileViewState } from '../hooks/use-file-view-state'
import { useSelectedFile } from '../hooks/use-selected-file'
import type { FileViewMode } from './file-viewer'
import { isPreviewable } from './file-viewer-utils'

interface FilesWorkspaceControlsProps {
  taskId: string
}

interface ModeButtonProps {
  mode: FileViewMode
  activeMode: FileViewMode
  onSelect: (mode: FileViewMode) => void
  disabled?: boolean
  label: string
  children: ReactNode
}

function ModeButton({
  mode,
  activeMode,
  onSelect,
  disabled = false,
  label,
  children,
}: ModeButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant={activeMode === mode ? 'secondary' : 'ghost'}
          size="icon"
          className="h-8 w-8"
          onClick={() => onSelect(mode)}
          disabled={disabled}
          aria-label={label}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

export function FilesWorkspaceControls({ taskId }: FilesWorkspaceControlsProps) {
  const { selectedFile } = useSelectedFile()
  const { fileView, setFileView } = useFileViewState()
  const { data } = useFileContent(taskId, selectedFile, selectedFile !== undefined)

  const fileOnlyHref = (() => {
    if (!selectedFile) {
      return
    }
    const params = new URLSearchParams({ file: selectedFile, fileView: 'preview' })
    return `/tasks/${encodeURIComponent(taskId)}/file?${params.toString()}`
  })()

  const hasSelectedFile = selectedFile !== undefined
  const canDiff = data?.isModified === true && data.oldContent !== undefined
  const canPreview = selectedFile ? isPreviewable(selectedFile) : false

  return (
    <div className="flex items-center gap-1">
      <div className="flex items-center rounded-md border overflow-hidden">
        <ModeButton
          mode="code"
          activeMode={fileView}
          onSelect={setFileView}
          disabled={!hasSelectedFile}
          label="Show code"
        >
          <Code className="h-4 w-4" />
        </ModeButton>
        <ModeButton
          mode="diff"
          activeMode={fileView}
          onSelect={setFileView}
          disabled={!(hasSelectedFile && canDiff)}
          label="Show diff"
        >
          <GitCompareArrows className="h-4 w-4" />
        </ModeButton>
        <ModeButton
          mode="preview"
          activeMode={fileView}
          onSelect={setFileView}
          disabled={!(hasSelectedFile && canPreview)}
          label="Show preview"
        >
          <Eye className="h-4 w-4" />
        </ModeButton>
      </div>

      {fileOnlyHref ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" className="h-8 w-8" asChild>
              <a
                href={fileOnlyHref}
                target="_blank"
                rel="noreferrer"
                aria-label="Open file-only preview"
              >
                <ExternalLink className="h-4 w-4" />
              </a>
            </Button>
          </TooltipTrigger>
          <TooltipContent>Open file-only preview</TooltipContent>
        </Tooltip>
      ) : (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              disabled
              aria-label="Open file-only preview"
            >
              <ExternalLink className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Open file-only preview</TooltipContent>
        </Tooltip>
      )}
    </div>
  )
}
