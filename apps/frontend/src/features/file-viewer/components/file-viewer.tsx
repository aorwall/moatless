import { Code2, GitCompare } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { oneDark, oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism'
import { getFileExtension, getLanguageFromPath, useFileContent } from '@/features/file-viewer'
import { useTheme } from '@/lib/components/theme-provider'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'
import { FileDiffViewer } from './file-diff-viewer'
import { FileViewerSkeleton } from './file-viewer-skeleton'
import { MarkdownPreview } from './markdown-preview'
import { type PreviewMode, PreviewModeToggle } from './preview-mode-toggle'

interface FileViewerProps {
  taskId: string
  filePath: string
}

export function FileViewer({ taskId, filePath }: FileViewerProps) {
  const { data, isLoading, error } = useFileContent(taskId, filePath)
  const { theme } = useTheme()
  const language = getLanguageFromPath(filePath)
  const fileExtension = getFileExtension(filePath).toLowerCase()

  // Determine if file supports preview mode
  const supportsPreview = fileExtension === 'md' || fileExtension === 'markdown'
  const [previewMode, setPreviewMode] = useState<PreviewMode>('code')

  // Determine if file has changes (for diff mode)
  const hasChanges = data?.isModified && data?.oldContent

  // Auto-set diff view when file has changes (derived state pattern)
  const initialViewMode = hasChanges ? 'diff' : 'code'
  const [viewMode, setViewMode] = useState<'code' | 'diff'>(initialViewMode)

  // Memoize content processing
  const content = useMemo(() => {
    if (!data?.content) {
      return ''
    }
    return String(data.content)
  }, [data?.content])

  // Determine syntax highlighter theme based on current theme
  const syntaxTheme = useMemo(() => {
    const isDark =
      theme === 'dark' ||
      (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)
    return isDark ? oneDark : oneLight
  }, [theme])

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
    <div className="file-viewer flex flex-col h-full min-w-0">
      <div className="file-header p-4 border-b flex items-center justify-between min-w-0">
        <h3 className="text-sm font-medium truncate flex-1 min-w-0">{filePath}</h3>
        <div className="flex items-center gap-2 flex-shrink-0">
          {hasChanges && (
            <ToggleGroup
              type="single"
              value={viewMode}
              onValueChange={(value) => value && setViewMode(value as 'code' | 'diff')}
              className="h-8"
            >
              <ToggleGroupItem value="code" aria-label="Code view" className="h-8 px-2">
                <Code2 className="h-4 w-4" />
              </ToggleGroupItem>
              <ToggleGroupItem value="diff" aria-label="Diff view" className="h-8 px-2">
                <GitCompare className="h-4 w-4" />
              </ToggleGroupItem>
            </ToggleGroup>
          )}
          {supportsPreview && viewMode === 'code' && (
            <PreviewModeToggle mode={previewMode} onModeChange={setPreviewMode} />
          )}
        </div>
      </div>
      <div className="file-content flex-1 overflow-auto min-w-0 max-w-full">
        {viewMode === 'diff' && hasChanges && data?.oldContent ? (
          <FileDiffViewer
            filePath={filePath}
            oldContent={data.oldContent}
            newContent={content}
            additions={data.additions}
            deletions={data.deletions}
          />
        ) : supportsPreview && previewMode === 'preview' ? (
          <MarkdownPreview content={content} />
        ) : (
          <div className="w-full max-w-full overflow-x-auto">
            <SyntaxHighlighter
              language={language}
              style={syntaxTheme}
              showLineNumbers
              lineNumberStyle={{
                minWidth: '3em',
                paddingRight: '1em',
                userSelect: 'none',
              }}
              customStyle={{
                margin: 0,
                padding: '1rem',
                background: 'transparent',
                fontSize: '0.875rem',
                width: 'max-content',
                minWidth: '100%',
              }}
              codeTagProps={{
                style: {
                  fontFamily: 'ui-monospace, monospace',
                },
              }}
              wrapLongLines={false}
            >
              {content}
            </SyntaxHighlighter>
          </div>
        )}
      </div>
    </div>
  )
}
