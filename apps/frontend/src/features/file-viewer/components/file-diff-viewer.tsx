import { Columns2, RowsIcon } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import ReactDiffViewer, { DiffMethod } from 'react-diff-viewer-continued'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { oneDark, oneLight } from 'react-syntax-highlighter/dist/esm/styles/prism'
import { useTheme } from '@/lib/components/theme-provider'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'
import { getLanguageFromPath } from '../utils/file-tree-utils'

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
  const [splitView, setSplitView] = useState(false)

  const resolvedTheme = useMemo(() => {
    if (theme === 'system') {
      return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
    }
    return theme
  }, [theme])

  const useDarkTheme = resolvedTheme === 'dark'
  const language = getLanguageFromPath(filePath)

  // Determine syntax highlighter theme based on current theme
  const syntaxTheme = useMemo(() => {
    return useDarkTheme ? oneDark : oneLight
  }, [useDarkTheme])

  // Create syntax highlighting function for diff content
  const renderContent = useCallback(
    (content: string) => {
      return (
        <SyntaxHighlighter
          language={language}
          style={syntaxTheme}
          customStyle={{
            margin: 0,
            padding: 0,
            background: 'transparent',
            fontSize: '0.875rem',
            fontFamily: 'ui-monospace, monospace',
          }}
          codeTagProps={{
            style: {
              fontFamily: 'ui-monospace, monospace',
            },
          }}
          PreTag="div"
        >
          {content}
        </SyntaxHighlighter>
      )
    },
    [language, syntaxTheme],
  )

  return (
    <div className="file-diff-viewer-v2 h-full flex flex-col">
      <div className="diff-header p-4 border-b flex items-center justify-between">
        <div className="diff-stats text-sm">
          <span className="text-status-success">+{additions} additions</span>
          <span className="mx-2">·</span>
          <span className="text-status-error">−{deletions} deletions</span>
        </div>
        <ToggleGroup
          type="single"
          value={splitView ? 'split' : 'unified'}
          onValueChange={(value) => setSplitView(value === 'split')}
          className="h-8"
        >
          <ToggleGroupItem value="unified" aria-label="Unified view" className="h-8 px-2">
            <RowsIcon className="h-4 w-4" />
          </ToggleGroupItem>
          <ToggleGroupItem value="split" aria-label="Split view" className="h-8 px-2">
            <Columns2 className="h-4 w-4" />
          </ToggleGroupItem>
        </ToggleGroup>
      </div>
      <div className="diff-content flex-1 overflow-auto">
        <div className="w-full" style={{ maxWidth: '100%' }}>
          <ReactDiffViewer
            oldValue={oldContent}
            newValue={newContent}
            splitView={splitView}
            useDarkTheme={useDarkTheme}
            compareMethod={DiffMethod.WORDS}
            hideLineNumbers={false}
            renderContent={renderContent}
          />
        </div>
      </div>
    </div>
  )
}
