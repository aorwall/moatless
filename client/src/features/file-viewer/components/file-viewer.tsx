import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { cn } from '@/lib/utils'
import { useFileContent } from '../api/use-file-content'
import { getLanguageFromPath } from '../utils/file-tree-utils'
import { FileViewerSkeleton } from './file-viewer-skeleton'

interface FileViewerProps {
  taskId: string
  filePath: string
}

export function FileViewer({ taskId, filePath }: FileViewerProps) {
  const { data, isLoading, error } = useFileContent(taskId, filePath)

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

  const language = getLanguageFromPath(filePath)
  const lines = data.content.split('\n')

  return (
    <div className="file-viewer flex flex-col h-full">
      <div className="file-header p-4 border-b flex items-center justify-between">
        <h3 className="text-sm font-medium truncate">{filePath}</h3>
        <span className="text-xs text-muted-foreground">{language}</span>
      </div>
      <div className="file-content flex-1 overflow-auto">
        <pre
          className={cn(
            'p-4 text-sm',
            'bg-gray-50 dark:bg-gray-900',
            'text-gray-800 dark:text-gray-200',
          )}
        >
          <code className={`language-${language}`}>
            {lines.map((line, index) => (
              <div key={index} className="table-row">
                <span className="table-cell pr-4 text-muted-foreground text-right select-none">
                  {index + 1}
                </span>
                <span className="table-cell">{line || '\n'}</span>
              </div>
            ))}
          </code>
        </pre>
      </div>
    </div>
  )
}
