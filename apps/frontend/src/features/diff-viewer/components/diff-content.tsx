import { type DiffFile, DiffFileCard } from '@moatless/diff-viewer'
import { useCallback, useEffect, useRef } from 'react'
import { ScrollArea } from '@/lib/components/ui/scroll-area'

interface DiffContentProps {
  files: DiffFile[]
  selectedFile: string | undefined
  isFileCollapsed: (filename: string) => boolean
  onToggleCollapsed: (filename: string) => void
}

/**
 * Main content area showing diff file cards
 */
export function DiffContent({
  files,
  selectedFile,
  isFileCollapsed,
  onToggleCollapsed,
}: DiffContentProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const fileRefs = useRef<Map<string, HTMLDivElement>>(new Map())

  // Scroll to selected file when it changes
  useEffect(() => {
    if (selectedFile && fileRefs.current.has(selectedFile)) {
      const element = fileRefs.current.get(selectedFile)
      element?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [selectedFile])

  const setFileRef = useCallback((filename: string, element: HTMLDivElement | null) => {
    if (element) {
      fileRefs.current.set(filename, element)
    } else {
      fileRefs.current.delete(filename)
    }
  }, [])

  if (files.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        No changes to display
      </div>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div ref={containerRef} className="p-4 space-y-4">
        {files.map((file) => (
          <div key={file.filename} ref={(el) => setFileRef(file.filename, el)}>
            <DiffFileCard
              file={file}
              collapsed={isFileCollapsed(file.filename)}
              onCollapseChange={() => onToggleCollapsed(file.filename)}
            />
          </div>
        ))}
      </div>
    </ScrollArea>
  )
}
