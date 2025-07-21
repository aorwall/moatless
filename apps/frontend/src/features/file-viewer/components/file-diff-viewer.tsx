import { DiffModeEnum, DiffView } from '@git-diff-view/react'
import { useMemo } from 'react'
import '@git-diff-view/react/styles/diff-view.css'
import { DiffFile, generateDiffFile } from '@git-diff-view/file'
import { useTheme } from '@/lib/components/theme-provider'

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
  const { theme } = useTheme()

  const resolvedTheme = useMemo(() => {
    if (theme === 'system') {
      return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
    }
    return theme
  }, [theme])

  const diffData = useMemo(() => {
    const _diffFile = generateDiffFile(filePath, oldContent, filePath, newContent, 'tsx', 'tsx')

    const instance = DiffFile.createInstance({
      oldFile: { content: oldContent, fileName: filePath },
      newFile: { content: newContent, fileName: filePath },
      hunks: _diffFile._diffList,
    })
    instance.initRaw()
    return instance
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
          diffFile={diffData}
          diffViewMode={DiffModeEnum.Unified}
          diffViewTheme={resolvedTheme}
          className="diff-view"
          diffViewHighlight={true}
        />
      </div>
    </div>
  )
}
