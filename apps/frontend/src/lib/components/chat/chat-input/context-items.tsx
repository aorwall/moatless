import type { ElementContextData, MessageContextInput } from '@moatless/api-client/sandbox/model'
import { ExternalLink, Eye, Figma, Loader2, X } from 'lucide-react'
import { useState } from 'react'
import { FigmaImportViewer, figmaApi } from '@/features/figma'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { cn } from '@/lib/utils'

interface ContextItemsProps {
  contexts: MessageContextInput[]
  onRemove?: (index: number) => void
  onPreviewUrlClick?: (path: string) => void
  isLoading?: boolean
}

interface ElementDetailPopoverProps {
  element: ElementContextData
  displayName: string
  onClose: () => void
}

function ElementDetailPopover({ element, displayName, onClose }: ElementDetailPopoverProps) {
  // Extract filename from path
  const filename = element.path.split('/').pop() || element.path

  return (
    <div className="w-80 max-w-[80vw]">
      <div className="flex items-center justify-between p-3 border-b">
        <h3 className="text-sm font-semibold">Element Details</h3>
        <Button variant="ghost" size="icon" onClick={onClose} className="h-6 w-6">
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="p-3 space-y-2 min-w-0">
        <div className="space-y-1.5">
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Component</span>
            <span className="font-mono truncate">&lt;{displayName}&gt;</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">File</span>
            <span className="font-mono truncate">{filename}</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Path</span>
            <span className="font-mono truncate text-right">{element.path}</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Location</span>
            <span className="font-mono">
              {element.line}:{element.column}
            </span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Component ID</span>
            <span className="font-mono text-xs truncate">{element.componentId}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

export function ContextItems({
  contexts,
  onRemove,
  onPreviewUrlClick,
  isLoading = false,
}: ContextItemsProps) {
  const [openPopoverId, setOpenPopoverId] = useState<string | null>(null)
  const [selectedFigmaImportId, setSelectedFigmaImportId] = useState<string | null>(null)

  if (contexts.length === 0 && !isLoading) {
    return null
  }

  // Separate Figma contexts from other contexts
  const figmaContexts = contexts.filter((ctx) => ctx.type === 'figma')
  const otherContexts = contexts.filter((ctx) => ctx.type !== 'figma')

  return (
    <div className="flex flex-col gap-2 mb-2">
      {/* Figma imports as thumbnails */}
      {(figmaContexts.length > 0 || isLoading) && (
        <div className="flex flex-wrap gap-2">
          {/* Loading placeholder */}
          {isLoading && (
            <div className="inline-flex h-12 w-12 items-center justify-center rounded-lg border border-border bg-muted">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            </div>
          )}

          {figmaContexts.map((context) => {
            if (context.type !== 'figma') {
              return null
            }
            const figma = context.data
            const imageUrl = figmaApi.getImageUrl(figma.importId)
            const contextIndex = contexts.indexOf(context)

            return (
              <div key={figma.importId} className="relative group w-fit">
                <button
                  type="button"
                  onClick={() => setSelectedFigmaImportId(figma.importId)}
                  className="block"
                >
                  {figma.hasImage ? (
                    <img
                      src={imageUrl}
                      alt={figma.nodeName}
                      className="h-12 w-12 object-cover rounded border-1 border-border hover:border-foreground/50 transition-colors cursor-pointer"
                    />
                  ) : (
                    <div
                      className="h-12 w-12 flex items-center justify-center rounded border-1 border-border hover:border-foreground/50 transition-colors cursor-pointer bg-secondary/50"
                      title={figma.nodeName}
                    >
                      <Figma className="h-6 w-6 text-muted-foreground" />
                    </div>
                  )}
                </button>
                {onRemove && (
                  <button
                    type="button"
                    onClick={() => onRemove(contextIndex)}
                    className="absolute top-1 right-1 bg-background/80 hover:bg-background rounded-full p-1 opacity-0 group-hover:opacity-100 transition-opacity"
                    aria-label="Remove Figma import"
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>
            )
          })}
        </div>
      )}

      {/* Other context types */}
      {otherContexts.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {/* biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex conditional rendering required for different context types */}
          {otherContexts.map((context) => {
            const contextIndex = contexts.indexOf(context)

            if (context.type === 'element') {
              const element = context.data
              const displayName = element.componentName || 'Unknown Element'
              const filename = element.path.split('/').pop() || element.path
              // Create a unique key from element properties
              const uniqueKey =
                element.componentId || `${element.path}-${element.line}-${contextIndex}`

              return (
                <Popover
                  key={uniqueKey}
                  open={openPopoverId === uniqueKey}
                  onOpenChange={(open) => setOpenPopoverId(open ? uniqueKey : null)}
                >
                  <div className="flex items-center gap-1.5 px-2 py-1 bg-secondary/10 rounded text-xs group">
                    <PopoverTrigger asChild>
                      <button
                        type="button"
                        className="flex items-center gap-1.5 cursor-pointer hover:opacity-80 transition-opacity"
                      >
                        <Eye className="h-3 w-3 text-muted-foreground" />
                        <span className="max-w-[150px] truncate" title={displayName}>
                          {displayName}
                        </span>
                        <span className="text-muted-foreground">
                          in {filename}:{element.line}
                        </span>
                      </button>
                    </PopoverTrigger>
                    {onRemove && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        onClick={(e) => {
                          e.stopPropagation()
                          onRemove(contextIndex)
                        }}
                        className="h-4 w-4 p-0 text-muted-foreground hover:text-foreground transition-colors opacity-0 group-hover:opacity-100"
                        aria-label="Remove context"
                      >
                        <X className="h-3 w-3" />
                      </Button>
                    )}
                  </div>
                  <PopoverContent align="start" className="p-0 w-auto">
                    <ElementDetailPopover
                      element={element}
                      displayName={displayName}
                      onClose={() => setOpenPopoverId(null)}
                    />
                  </PopoverContent>
                </Popover>
              )
            }

            if (context.type === 'previewUrl') {
              const previewUrl = context.data
              const displayPath = previewUrl.path || previewUrl.url || 'Unknown'
              const uniqueKey = `preview-${contextIndex}`

              return (
                // biome-ignore lint/a11y/noStaticElementInteractions: Div has proper role, tabIndex, and keyboard handler
                <div
                  key={uniqueKey}
                  role={onPreviewUrlClick ? 'button' : undefined}
                  tabIndex={onPreviewUrlClick ? 0 : undefined}
                  onClick={onPreviewUrlClick ? () => onPreviewUrlClick(displayPath) : undefined}
                  onKeyDown={
                    onPreviewUrlClick
                      ? (e) => {
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault()
                            onPreviewUrlClick(displayPath)
                          }
                        }
                      : undefined
                  }
                  className={cn(
                    'flex items-center gap-1.5 px-2 py-1 bg-secondary/10 rounded text-xs group',
                    onPreviewUrlClick && 'cursor-pointer hover:bg-secondary/20 transition-colors',
                  )}
                >
                  <ExternalLink className="h-3 w-3 text-muted-foreground" />
                  <span className="max-w-[200px] truncate" title={displayPath}>
                    {displayPath}
                  </span>
                  {onRemove && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation()
                        onRemove(contextIndex)
                      }}
                      className="h-4 w-4 p-0 text-muted-foreground hover:text-foreground transition-colors opacity-0 group-hover:opacity-100"
                      aria-label="Remove preview URL"
                    >
                      <X className="h-3 w-3" />
                    </Button>
                  )}
                </div>
              )
            }

            // Handle other context types in the future
            return null
          })}
        </div>
      )}

      {/* Figma import viewer dialog */}
      <FigmaImportViewer
        importId={selectedFigmaImportId}
        onClose={() => setSelectedFigmaImportId(null)}
      />
    </div>
  )
}
