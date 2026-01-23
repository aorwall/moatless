import type { ScreenshotMetadata } from '@moatless/types'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Camera,
  ChevronLeft,
  ChevronRight,
  Download,
  Loader2,
  Trash2,
  X,
  ZoomIn,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { browserApi } from '@/features/preview/api/server'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'

interface ScreenshotsTabProps {
  taskId: string | undefined
}

/**
 * Format action type for display
 */
function formatAction(action: ScreenshotMetadata['action']): string {
  switch (action) {
    case 'navigate':
      return 'Navigate'
    case 'manual':
      return 'Manual'
    default:
      return action
  }
}

/**
 * Get action badge color
 */
function getActionColor(action: ScreenshotMetadata['action']): string {
  switch (action) {
    case 'navigate':
      return 'bg-blue-500/20 text-blue-400'
    case 'manual':
      return 'bg-gray-500/20 text-gray-400'
    default:
      return 'bg-gray-500/20 text-gray-400'
  }
}

/**
 * Format details object for display
 */
function formatDetails(details: Record<string, unknown>): string {
  return Object.entries(details)
    .map(([key, value]) => `${key}: ${String(value)}`)
    .join(' \u2022 ')
}

export function ScreenshotsTab({ taskId }: ScreenshotsTabProps) {
  const queryClient = useQueryClient()
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const [shouldAutoScroll, setShouldAutoScroll] = useState(true)
  const prevLengthRef = useRef(0)

  // Fetch screenshots via backend API
  const { data, isLoading, error } = useQuery({
    queryKey: ['browser-screenshots', taskId],
    queryFn: () => browserApi.getScreenshots(taskId!),
    enabled: !!taskId,
    refetchInterval: 2000, // Poll every 2 seconds for new screenshots
  })

  const screenshots = data?.screenshots ?? []

  // Derive selected screenshot from index
  const selectedScreenshot = selectedIndex !== null ? screenshots[selectedIndex] : null

  // Navigation handlers
  const handlePrevious = useCallback(() => {
    if (selectedIndex !== null && selectedIndex > 0) {
      setSelectedIndex(selectedIndex - 1)
    }
  }, [selectedIndex])

  const handleNext = useCallback(() => {
    if (selectedIndex !== null && selectedIndex < screenshots.length - 1) {
      setSelectedIndex(selectedIndex + 1)
    }
  }, [selectedIndex, screenshots.length])

  const handleClose = useCallback(() => {
    setSelectedIndex(null)
  }, [])

  // Auto-scroll to bottom when new screenshots arrive
  useEffect(() => {
    if (shouldAutoScroll && screenshots.length > prevLengthRef.current) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
    }
    prevLengthRef.current = screenshots.length
  }, [screenshots.length, shouldAutoScroll])

  // Handle scroll to detect if user scrolled up
  const handleScroll = useCallback(() => {
    if (!scrollAreaRef.current) {
      return
    }
    const element = scrollAreaRef.current
    const isAtBottom = element.scrollHeight - element.scrollTop <= element.clientHeight + 50
    setShouldAutoScroll(isAtBottom)
  }, [])

  // Clear all screenshots
  const handleClear = useCallback(async () => {
    if (!taskId) {
      return
    }
    try {
      await browserApi.clearScreenshots(taskId)
      queryClient.invalidateQueries({ queryKey: ['browser-screenshots', taskId] })
    } catch {
      // Ignore errors
    }
  }, [taskId, queryClient])

  // Download screenshot
  const handleDownload = useCallback(
    (screenshot: ScreenshotMetadata) => {
      if (!taskId) {
        return
      }
      const url = browserApi.getScreenshotUrl(taskId, screenshot.filename)
      const link = document.createElement('a')
      link.href = url
      link.download = screenshot.filename
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)
    },
    [taskId],
  )

  // No task ID
  if (!taskId) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <Camera className="h-12 w-12 mx-auto mb-4 opacity-50" />
          <p className="text-lg font-medium">No task selected</p>
          <p className="mt-2 text-sm">Select a task to view screenshots</p>
        </div>
      </div>
    )
  }

  // Loading state
  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  // Error state
  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">Failed to load screenshots</p>
          <p className="mt-2 text-sm">{error.message}</p>
        </div>
      </div>
    )
  }

  // Empty state
  if (screenshots.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <Camera className="h-12 w-12 mx-auto mb-4 opacity-50" />
          <p className="text-lg font-medium">No screenshots yet</p>
          <p className="mt-2 text-sm">
            Screenshots can be captured manually via captureScreenshot()
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-background relative flex flex-col">
      {/* Header with clear button */}
      <div className="flex items-center justify-between px-4 py-2 border-b">
        <span className="text-xs text-muted-foreground">
          {screenshots.length} screenshot{screenshots.length !== 1 ? 's' : ''}
        </span>
        <Button
          variant="ghost"
          size="sm"
          onClick={handleClear}
          className="h-7 text-xs text-muted-foreground hover:text-destructive"
        >
          <Trash2 className="h-3 w-3 mr-1" />
          Clear
        </Button>
      </div>

      {/* Screenshot grid */}
      <ScrollArea className="flex-1" onScrollCapture={handleScroll}>
        <div
          ref={scrollAreaRef}
          className="p-4 grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4"
        >
          {screenshots.map((screenshot, index) => (
            <button
              type="button"
              key={screenshot.id}
              className="group relative bg-muted rounded-lg overflow-hidden cursor-pointer hover:ring-2 hover:ring-primary transition-all text-left w-full"
              onClick={() => setSelectedIndex(index)}
            >
              {/* Thumbnail */}
              <div className="aspect-video relative">
                <img
                  src={browserApi.getScreenshotUrl(taskId, screenshot.filename)}
                  alt={`Screenshot: ${screenshot.action}`}
                  className="w-full h-full object-cover"
                  loading="lazy"
                />
                {/* Hover overlay */}
                <div className="absolute inset-0 bg-black/50 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                  <ZoomIn className="h-6 w-6 text-white" />
                </div>
              </div>

              {/* Metadata */}
              <div className="p-2 space-y-1">
                <div className="flex items-center gap-2">
                  <span
                    className={cn(
                      'text-xs px-1.5 py-0.5 rounded',
                      getActionColor(screenshot.action),
                    )}
                  >
                    {formatAction(screenshot.action)}
                  </span>
                  <span className="text-xs text-muted-foreground truncate">
                    {new Date(screenshot.timestamp).toLocaleTimeString()}
                  </span>
                </div>
                <p className="text-xs text-muted-foreground truncate" title={screenshot.url}>
                  {screenshot.url}
                </p>
              </div>
            </button>
          ))}
          <div ref={bottomRef} />
        </div>
      </ScrollArea>

      {/* Lightbox modal */}
      {selectedScreenshot && selectedIndex !== null && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={`Screenshot preview: ${selectedScreenshot.action}`}
          className="fixed inset-0 z-50 bg-black/80 flex items-center justify-center p-4"
          onClick={handleClose}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              handleClose()
            }
            if (e.key === 'ArrowLeft') {
              handlePrevious()
            }
            if (e.key === 'ArrowRight') {
              handleNext()
            }
          }}
        >
          <div
            role="document"
            className="relative max-w-[90vw] max-h-[90vh] bg-background rounded-lg overflow-hidden flex flex-col"
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
          >
            {/* Close button */}
            <button
              type="button"
              onClick={handleClose}
              className="absolute top-2 right-2 z-10 p-2 rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors"
            >
              <X className="h-5 w-5" />
            </button>

            {/* Image container with navigation buttons */}
            <div className="relative flex items-center">
              {/* Previous button */}
              <button
                type="button"
                onClick={handlePrevious}
                disabled={selectedIndex === 0}
                className="absolute left-2 z-10 p-2 rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                aria-label="Previous screenshot"
              >
                <ChevronLeft className="h-6 w-6" />
              </button>

              {/* Image */}
              <img
                src={browserApi.getScreenshotUrl(taskId, selectedScreenshot.filename)}
                alt={`Screenshot: ${selectedScreenshot.action}`}
                className="max-w-full max-h-[70vh] object-contain"
              />

              {/* Next button */}
              <button
                type="button"
                onClick={handleNext}
                disabled={selectedIndex === screenshots.length - 1}
                className="absolute right-2 z-10 p-2 rounded-full bg-black/50 text-white hover:bg-black/70 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                aria-label="Next screenshot"
              >
                <ChevronRight className="h-6 w-6" />
              </button>
            </div>

            {/* Footer with metadata, details, and download */}
            <div className="p-4 border-t space-y-3">
              <div className="flex items-start justify-between gap-4">
                <div className="space-y-1 min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span
                      className={cn(
                        'text-xs px-1.5 py-0.5 rounded',
                        getActionColor(selectedScreenshot.action),
                      )}
                    >
                      {formatAction(selectedScreenshot.action)}
                    </span>
                    <span className="text-sm text-muted-foreground">
                      {new Date(selectedScreenshot.timestamp).toLocaleString()}
                    </span>
                  </div>
                  <p
                    className="text-sm text-muted-foreground truncate"
                    title={selectedScreenshot.url}
                  >
                    {selectedScreenshot.url}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {selectedScreenshot.width} x {selectedScreenshot.height}
                  </p>
                </div>
                <div className="flex items-center gap-3 shrink-0">
                  <span className="text-sm text-muted-foreground">
                    {selectedIndex + 1} / {screenshots.length}
                  </span>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleDownload(selectedScreenshot)}
                  >
                    <Download className="h-4 w-4 mr-2" />
                    Download
                  </Button>
                </div>
              </div>

              {/* Action details */}
              {selectedScreenshot.details && Object.keys(selectedScreenshot.details).length > 0 && (
                <div
                  className="text-xs text-muted-foreground font-mono bg-muted p-2 rounded truncate"
                  title={formatDetails(selectedScreenshot.details)}
                >
                  {formatDetails(selectedScreenshot.details)}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
