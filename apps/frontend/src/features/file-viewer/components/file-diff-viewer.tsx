import { DiffModeEnum, DiffView } from '@git-diff-view/react'
import { useMemo } from 'react'
import '@git-diff-view/react/styles/diff-view.css'

interface FileDiffViewerProps {
  filePath: string
  oldContent: string
  newContent: string
  additions?: number
  deletions?: number
}

export function FileDiffViewer({
  filePath,
  oldContent,
  newContent,
  additions = 0,
  deletions = 0,
}: FileDiffViewerProps) {
  const diffData = useMemo(() => {
    return {
      oldFile: {
        fileName: filePath,
        content: oldContent,
      },
      newFile: {
        fileName: filePath,
        content: newContent,
      },
      hunks: [],
    }
  }, [filePath, oldContent, newContent])

  return (
    <div className="file-diff-viewer h-full overflow-auto">
      <div className="diff-stats p-4 border-b text-sm">
        <span className="text-status-success">+{additions} additions</span>
        <span className="mx-2">·</span>
        <span className="text-status-error">−{deletions} deletions</span>
      </div>
      <div className="diff-content">
        <DiffView
          data={diffData}
          diffViewMode={DiffModeEnum.Unified}
          diffViewTheme="light"
          className="diff-view"
        />
      </div>
    </div>
  )
}
