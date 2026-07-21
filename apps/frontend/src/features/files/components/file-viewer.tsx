import { File as PierreFile } from '@pierre/diffs/react'
import { FileQuestion, Music } from 'lucide-react'
import { type ReactNode, useEffect } from 'react'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { useFileContent } from '../hooks/use-file-content'
import { usePierreOptions } from '../lib/use-pierre-theme'
import { DiffFileView } from './diff-file-view'
import { FileModeToggleGroup } from './file-mode-toggle'
import { isHtmlFile, isMarkdownFile, isPreviewable } from './file-viewer-utils'
import { HtmlPreview } from './html-preview'
import { MarkdownPreview } from './markdown-preview'

export type FileViewMode = 'code' | 'diff' | 'preview'

const FILE_VIEWER_SKELETON_WIDTHS = [
  42, 78, 55, 87, 64, 35, 71, 49, 82, 59, 90, 46, 74, 61, 38, 85, 52, 68, 44, 80,
] as const

function isImageMediaType(mediaType: string | undefined): boolean {
  return mediaType?.startsWith('image/') ?? false
}

function isAudioMediaType(mediaType: string | undefined): boolean {
  return mediaType?.startsWith('audio/') ?? false
}

function getLanguageLabel(filePath: string): string {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  const map: Record<string, string> = {
    ts: 'TypeScript',
    tsx: 'TypeScript (JSX)',
    js: 'JavaScript',
    jsx: 'JavaScript (JSX)',
    py: 'Python',
    rb: 'Ruby',
    go: 'Go',
    rs: 'Rust',
    java: 'Java',
    css: 'CSS',
    html: 'HTML',
    json: 'JSON',
    yaml: 'YAML',
    yml: 'YAML',
    md: 'Markdown',
    sh: 'Shell',
    bash: 'Shell',
    sql: 'SQL',
    toml: 'TOML',
    xml: 'XML',
    svg: 'SVG',
    dockerfile: 'Dockerfile',
  }
  return map[ext] || ext.toUpperCase() || 'Plain Text'
}

function BinaryFileView({
  content,
  mediaType,
  filePath,
}: {
  content: string
  mediaType?: string | undefined
  filePath: string
}) {
  if (isImageMediaType(mediaType)) {
    return (
      <div className="flex items-center justify-center p-8">
        <img
          src={`data:${mediaType};base64,${content}`}
          alt={filePath}
          className="max-w-full max-h-full object-contain"
        />
      </div>
    )
  }

  if (isAudioMediaType(mediaType)) {
    return (
      <div className="flex flex-col items-center justify-center p-8 gap-4">
        <div className="flex items-center gap-2">
          <Music className="w-5 h-5 text-muted-foreground" />
          <span className="text-sm font-mono">{filePath.split('/').pop()}</span>
        </div>
        {/* biome-ignore lint/a11y/useMediaCaption: audio files from filesystem have no captions */}
        <audio controls src={`data:${mediaType};base64,${content}`} className="w-full max-w-md">
          Your browser does not support audio playback.
        </audio>
      </div>
    )
  }

  return (
    <div className="h-full flex items-center justify-center text-muted-foreground">
      <div className="text-center space-y-2">
        <FileQuestion className="w-10 h-10 mx-auto opacity-40" />
        <p className="text-sm">Binary file — cannot display</p>
        {mediaType && <p className="text-xs text-muted-foreground">{mediaType}</p>}
      </div>
    </div>
  )
}

function FileContentArea({
  data,
  filePath,
  viewMode,
  taskId,
}: {
  data: {
    content: string
    oldContent?: string
    isBinary?: boolean
    mediaType?: string
    isModified?: boolean
  }
  filePath: string
  viewMode: FileViewMode
  taskId: string
}) {
  const isMarkdown = isMarkdownFile(filePath)
  const isHtml = isHtmlFile(filePath)
  const canDiff = data.isModified && data.oldContent !== undefined
  const pierreOptions = usePierreOptions()

  if (data.isBinary) {
    return <BinaryFileView content={data.content} mediaType={data.mediaType} filePath={filePath} />
  }

  if (viewMode === 'preview' && isMarkdown) {
    return <MarkdownPreview content={data.content} taskId={taskId} filePath={filePath} />
  }

  if (viewMode === 'preview' && isHtml) {
    return <HtmlPreview content={data.content} filePath={filePath} />
  }

  if (viewMode === 'diff' && canDiff) {
    return (
      <DiffFileView
        oldContent={data.oldContent ?? ''}
        newContent={data.content}
        filename={filePath}
      />
    )
  }

  return (
    <PierreFile
      file={{ name: filePath, contents: data.content }}
      options={{ ...pierreOptions, disableFileHeader: true }}
    />
  )
}

interface FileViewerProps {
  taskId: string
  filePath: string | undefined
  viewMode: FileViewMode
  onViewModeChange: (viewMode: FileViewMode) => void
  hideHeaderActions?: boolean
  hideViewModeControls?: boolean
  /** Optional control rendered at the start of the header row (e.g. the file
   *  tree expand toggle when the explorer is collapsed). */
  leadingControl?: ReactNode
}

interface FileViewerHeaderProps {
  filePath: string | undefined
  isModified: boolean
  additions: number | undefined
  deletions: number | undefined
  viewMode: FileViewMode
  onViewModeChange: (viewMode: FileViewMode) => void
  canDiff: boolean
  canPreview: boolean
  isMobile: boolean
  hideHeaderActions: boolean
  hideViewModeControls: boolean
  leadingControl?: ReactNode
}

function FileViewerHeader({
  filePath,
  isModified,
  additions,
  deletions,
  viewMode,
  onViewModeChange,
  canDiff,
  canPreview,
  isMobile,
  hideHeaderActions,
  hideViewModeControls,
  leadingControl,
}: FileViewerHeaderProps) {
  const showToggle = !hideViewModeControls && (canDiff || canPreview)

  return (
    <div className="h-[42px] px-2 flex-shrink-0 flex items-center justify-between gap-2 border-b">
      <div className="flex items-center gap-2 min-w-0 flex-1">
        {leadingControl}
        {filePath ? (
          <span
            className="min-w-0 flex-1 text-sm font-mono truncate"
            // `direction: rtl` moves the ellipsis to the left so the file name
            // (end of the path) stays visible; the LTR isolate chars keep the
            // path itself reading left-to-right.
            style={{ direction: 'rtl', textAlign: 'left' }}
            title={filePath}
          >
            {`\u2066${filePath}\u2069`}
          </span>
        ) : (
          <span className="text-sm text-muted-foreground truncate">No file selected</span>
        )}
        {filePath && !isMobile && (
          <span className="text-xs px-1.5 py-0.5 rounded shrink-0 bg-muted text-muted-foreground">
            {getLanguageLabel(filePath)}
          </span>
        )}
      </div>
      {filePath && !hideHeaderActions && (
        <div className="flex items-center gap-2 shrink-0 ml-2">
          {isModified && (
            <div className="flex items-center gap-2 text-xs">
              <span style={{ color: 'var(--git-added)' }}>+{additions}</span>
              <span style={{ color: 'var(--git-deleted)' }}>&minus;{deletions}</span>
            </div>
          )}
          {showToggle && (
            <FileModeToggleGroup
              viewMode={viewMode}
              onViewModeChange={onViewModeChange}
              canDiff={canDiff}
              canPreview={canPreview}
            />
          )}
        </div>
      )}
    </div>
  )
}

export function FileViewer({
  taskId,
  filePath,
  viewMode,
  onViewModeChange,
  hideHeaderActions = false,
  hideViewModeControls = false,
  leadingControl,
}: FileViewerProps) {
  const isMobile = useIsMobile()
  const { data, isLoading, error } = useFileContent(taskId, filePath, true)
  const canPreview = filePath ? isPreviewable(filePath) : false
  const canDiff = data?.isModified && data?.oldContent !== undefined

  useEffect(() => {
    if (viewMode === 'diff' && !isLoading && !canDiff) {
      onViewModeChange('code')
    }
    if (viewMode === 'preview' && !canPreview) {
      onViewModeChange('code')
    }
  }, [viewMode, isLoading, canDiff, canPreview, onViewModeChange])

  const header = (
    <FileViewerHeader
      filePath={filePath}
      isModified={data?.isModified ?? false}
      additions={data?.additions}
      deletions={data?.deletions}
      viewMode={viewMode}
      onViewModeChange={onViewModeChange}
      canDiff={Boolean(canDiff)}
      canPreview={canPreview}
      isMobile={isMobile}
      hideHeaderActions={hideHeaderActions}
      hideViewModeControls={hideViewModeControls}
      leadingControl={leadingControl}
    />
  )

  let body: ReactNode
  if (!filePath) {
    body = (
      <div className="h-full flex items-center justify-center text-muted-foreground">
        <div className="text-center space-y-2">
          <div className="text-4xl opacity-30">{'{ }'}</div>
          <p className="text-sm">Select a file to view its contents</p>
        </div>
      </div>
    )
  } else if (isLoading) {
    body = (
      <div className="p-4 space-y-2">
        {FILE_VIEWER_SKELETON_WIDTHS.map((width) => (
          <Skeleton key={width} className="h-4" style={{ width: `${width}%` }} />
        ))}
      </div>
    )
  } else if (error) {
    body = (
      <div className="h-full flex items-center justify-center">
        <div className="text-center space-y-2 text-destructive">
          <p className="text-sm font-medium">Failed to load file</p>
          <p className="text-xs">{error.message}</p>
        </div>
      </div>
    )
  } else if (data) {
    body = <FileContentArea data={data} filePath={filePath} viewMode={viewMode} taskId={taskId} />
  }

  return (
    <div className="h-full flex flex-col">
      {header}
      <div className="flex-1 min-h-0 overflow-auto">{body}</div>
    </div>
  )
}
