import { useMemo, useState } from 'react'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { oneDark, oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism'
import { useTheme } from '@/lib/components/theme-provider'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { useFileContent } from '../api/use-file-content'
import { getLanguageFromPath, getFileExtension } from '../utils/file-tree-utils'
import { FileViewerSkeleton } from './file-viewer-skeleton'
import { MarkdownPreview } from './markdown-preview'
import { PreviewModeToggle, type PreviewMode } from './preview-mode-toggle'

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

  // Memoize content processing
  const content = useMemo(() => {
    if (!data?.content) return ''
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
          {supportsPreview && (
            <PreviewModeToggle mode={previewMode} onModeChange={setPreviewMode} />
          )}
          <span className="text-xs text-muted-foreground">{language}</span>
        </div>
      </div>
      <div className="file-content flex-1 overflow-auto min-w-0">
        {supportsPreview && previewMode === 'preview' ? (
          <MarkdownPreview content={content} />
        ) : (
          <SyntaxHighlighter
            language={language}
            style={syntaxTheme}
            showLineNumbers
            lineNumberStyle={{ minWidth: '3em', paddingRight: '1em', userSelect: 'none' }}
            customStyle={{
              margin: 0,
              padding: '1rem',
              background: 'transparent',
              fontSize: '0.875rem',
              maxWidth: '100%',
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
        )}
      </div>
    </div>
  )
}
