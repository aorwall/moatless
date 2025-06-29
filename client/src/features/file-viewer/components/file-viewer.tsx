import { useMemo } from 'react'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { useFileContent } from '../api/use-file-content'
import { getLanguageFromPath } from '../utils/file-tree-utils'
import { FileViewerSkeleton } from './file-viewer-skeleton'

interface FileViewerProps {
  taskId: string
  filePath: string
}

export function FileViewer({ taskId, filePath }: FileViewerProps) {
  const { data, isLoading, error } = useFileContent(taskId, filePath)
  const language = getLanguageFromPath(filePath)

  // Memoize line processing to avoid re-computation and ensure consistency
  // This hook must be called unconditionally to satisfy React's rules of hooks
  const lines = useMemo(() => {
    if (!data?.content) return []

    // Create a defensive copy to ensure we don't affect cached data
    const contentStr = String(data.content)

    // Normalize line endings: CRLF -> LF, CR -> LF
    const normalized = contentStr.replace(/\r\n/g, '\n').replace(/\r/g, '\n')

    return normalized.split('\n')
  }, [data?.content])

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <FileViewerSkeleton />
  }

  if (!data?.content) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        File is empty
      </div>
    )
  }

  return (
    <div className="file-viewer flex flex-col h-full">
      <div className="file-header p-4 border-b flex items-center justify-between">
        <h3 className="text-sm font-medium truncate">{filePath}</h3>
        <span className="text-xs text-muted-foreground">{language}</span>
      </div>
      <div className="file-content flex-1 overflow-auto">
        <div className="p-4">
          <pre className="text-sm text-foreground">
            <code className={`language-${language}`}>
              {lines.map((line, index) => (
                <div key={index} className="grid grid-cols-[auto_1fr] gap-4">
                  <span className="text-muted-foreground text-right select-none">{index + 1}</span>
                  <span className="whitespace-pre-wrap break-words">{line || '\u00A0'}</span>
                </div>
              ))}
            </code>
          </pre>
        </div>
      </div>
    </div>
  )
}
