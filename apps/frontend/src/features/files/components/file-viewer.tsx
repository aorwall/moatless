import { File as PierreFile } from '@pierre/diffs/react'
import { Code, Eye, FileQuestion, GitCompareArrows, Music } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useFileContent } from '../hooks/use-file-content'
import { usePierreOptions } from '../lib/use-pierre-theme'
import { DiffFileView } from './diff-file-view'
import { HtmlPreview } from './html-preview'
import { MarkdownPreview } from './markdown-preview'

type ViewMode = 'code' | 'diff' | 'preview'

function isMarkdownFile(filePath: string): boolean {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  return ext === 'md' || ext === 'mdx'
}

function isHtmlFile(filePath: string): boolean {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  return ext === 'html' || ext === 'htm'
}

function isPreviewable(filePath: string): boolean {
  return isMarkdownFile(filePath) || isHtmlFile(filePath)
}

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
  viewMode: ViewMode
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
}

export function FileViewer({ taskId, filePath }: FileViewerProps) {
  const [viewMode, setViewMode] = useState<ViewMode>('code')
  const { data, isLoading, error } = useFileContent(taskId, filePath, true)
  const canPreview = filePath ? isPreviewable(filePath) : false
  const canDiff = data?.isModified && data?.oldContent !== undefined

  useEffect(() => {
    if (viewMode === 'diff' && !canDiff) {
      setViewMode('code')
    }
    if (viewMode === 'preview' && !canPreview) {
      setViewMode('code')
    }
  }, [viewMode, canDiff, canPreview])

  if (!filePath) {
    return (
      <div className="h-full flex items-center justify-center text-muted-foreground">
        <div className="text-center space-y-2">
          <div className="text-4xl opacity-30">{'{ }'}</div>
          <p className="text-sm">Select a file to view its contents</p>
        </div>
      </div>
    )
  }

  if (isLoading) {
    return (
      <div className="h-full flex flex-col">
        <div className="p-3 flex-shrink-0 border-b">
          <Skeleton className="h-4 w-48" />
        </div>
        <div className="flex-1 p-4 space-y-2">
          {Array.from({ length: 20 }).map((_, i) => (
            <Skeleton
              // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton list
              key={i}
              className="h-4"
              style={{ width: `${30 + Math.random() * 60}%` }}
            />
          ))}
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-center space-y-2 text-destructive">
          <p className="text-sm font-medium">Failed to load file</p>
          <p className="text-xs">{error.message}</p>
        </div>
      </div>
    )
  }

  if (!data) {
    return null
  }

  const languageLabel = getLanguageLabel(filePath)
  const showToggle = canDiff || canPreview

  return (
    <div className="h-full flex flex-col">
      <div className="p-3 flex-shrink-0 flex items-center justify-between border-b">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-mono truncate">{filePath}</span>
          <span className="text-xs px-1.5 py-0.5 rounded shrink-0 bg-muted text-muted-foreground">
            {languageLabel}
          </span>
        </div>
        <div className="flex items-center gap-2 shrink-0 ml-2">
          {data.isModified && (
            <div className="flex items-center gap-2 text-xs">
              <span style={{ color: 'var(--git-added)' }}>+{data.additions}</span>
              <span style={{ color: 'var(--git-deleted)' }}>&minus;{data.deletions}</span>
            </div>
          )}
          {showToggle && (
            <div className="flex items-center rounded-md overflow-hidden border">
              <button
                type="button"
                onClick={() => setViewMode('code')}
                className={cn(
                  'flex items-center gap-1 px-2 py-1 text-xs transition-colors',
                  viewMode === 'code'
                    ? 'text-foreground bg-muted'
                    : 'text-muted-foreground hover:text-foreground',
                )}
              >
                <Code className="w-3.5 h-3.5" />
                Code
              </button>
              {canDiff && (
                <button
                  type="button"
                  onClick={() => setViewMode('diff')}
                  className={cn(
                    'flex items-center gap-1 px-2 py-1 text-xs transition-colors',
                    viewMode === 'diff'
                      ? 'text-foreground bg-muted'
                      : 'text-muted-foreground hover:text-foreground',
                  )}
                >
                  <GitCompareArrows className="w-3.5 h-3.5" />
                  Diff
                </button>
              )}
              {canPreview && (
                <button
                  type="button"
                  onClick={() => setViewMode('preview')}
                  className={cn(
                    'flex items-center gap-1 px-2 py-1 text-xs transition-colors',
                    viewMode === 'preview'
                      ? 'text-foreground bg-muted'
                      : 'text-muted-foreground hover:text-foreground',
                  )}
                >
                  <Eye className="w-3.5 h-3.5" />
                  Preview
                </button>
              )}
            </div>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-auto">
        <FileContentArea data={data} filePath={filePath} viewMode={viewMode} taskId={taskId} />
      </div>
    </div>
  )
}
