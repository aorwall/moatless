import type { ElementContextData, MessageContext } from '@moatless/types'
import { Eye, X } from 'lucide-react'
import { useState } from 'react'
import { figmaApi } from '@/features/figma'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

interface ContextItemsProps {
  contexts: MessageContext[]
  onRemove?: (index: number) => void
}

interface ElementDetailPopoverProps {
  element: ElementContextData
  displayName: string
  onClose: () => void
}

function ElementDetailPopover({ element, displayName, onClose }: ElementDetailPopoverProps) {
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
            <span className="text-muted-foreground flex-shrink-0">Element</span>
            <span className="font-mono truncate">&lt;{displayName}&gt;</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">File</span>
            <span className="font-mono truncate">{element.file}</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Path</span>
            <span className="font-mono truncate text-right">{element.path}</span>
          </div>
          <div className="flex justify-between text-xs gap-2 min-w-0">
            <span className="text-muted-foreground flex-shrink-0">Line</span>
            <span className="font-mono">{element.lineNumber}</span>
          </div>
          {element.sourceId && (
            <div className="flex justify-between text-xs gap-2 min-w-0">
              <span className="text-muted-foreground flex-shrink-0">Source ID</span>
              <span className="font-mono text-xs truncate">{element.sourceId}</span>
            </div>
          )}
          {element.componentName && (
            <div className="flex justify-between text-xs gap-2 min-w-0">
              <span className="text-muted-foreground flex-shrink-0">Component</span>
              <span className="font-mono truncate">{element.componentName}</span>
            </div>
          )}
          {element.elementType && (
            <div className="flex justify-between text-xs gap-2 min-w-0">
              <span className="text-muted-foreground flex-shrink-0">Type</span>
              <span className="font-mono truncate">{element.elementType}</span>
            </div>
          )}
          {element.className && (
            <div className="flex justify-between text-xs gap-2 min-w-0">
              <span className="text-muted-foreground flex-shrink-0">Class</span>
              <span className="font-mono truncate">{element.className}</span>
            </div>
          )}
          {element.content && (
            <div className="flex flex-col text-xs gap-1 min-w-0">
              <span className="text-muted-foreground">Content</span>
              <span className="font-mono text-xs p-1.5 bg-muted rounded break-words">
                {element.content}
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export function ContextItems({ contexts, onRemove }: ContextItemsProps) {
  const [openPopoverId, setOpenPopoverId] = useState<string | null>(null)
  const [selectedFigmaImage, setSelectedFigmaImage] = useState<{
    url: string
    fileName: string
    nodeName: string
  } | null>(null)

  if (contexts.length === 0) {
    return null
  }

  // Separate Figma contexts from other contexts
  const figmaContexts = contexts.filter((ctx) => ctx.type === 'figma')
  const otherContexts = contexts.filter((ctx) => ctx.type !== 'figma')

  return (
    <div className="flex flex-col gap-2 mb-2">
      {/* Figma imports as thumbnails */}
      {figmaContexts.length > 0 && (
        <div className="flex flex-wrap gap-2">
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
                  onClick={() =>
                    setSelectedFigmaImage({
                      url: imageUrl,
                      fileName: figma.fileName,
                      nodeName: figma.nodeName,
                    })
                  }
                  className="block"
                >
                  <img
                    src={imageUrl}
                    alt={figma.nodeName}
                    className="h-12 w-12 object-cover rounded border-1 border-border hover:border-foreground/50 transition-colors cursor-pointer"
                  />
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
          {otherContexts.map((context) => {
            const contextIndex = contexts.indexOf(context)

            if (context.type === 'element') {
              const element = context.data
              const displayName = element.componentName || element.elementType || 'Unknown Element'
              // Create a unique key from element properties
              const uniqueKey =
                element.sourceId || `${element.path}-${element.lineNumber}-${contextIndex}`

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
                          in {element.file}:{element.lineNumber}
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

            // Handle other context types in the future
            return null
          })}
        </div>
      )}

      {/* Full-size Figma image dialog */}
      <Dialog
        open={selectedFigmaImage !== null}
        onOpenChange={(open) => !open && setSelectedFigmaImage(null)}
      >
        <DialogContent className="max-h-[85vh] w-[90vw] max-w-4xl sm:max-w-4xl p-6">
          {selectedFigmaImage && (
            <>
              <DialogHeader>
                <DialogTitle className="truncate">
                  {selectedFigmaImage.fileName} - {selectedFigmaImage.nodeName}
                </DialogTitle>
                <DialogDescription>Figma Import</DialogDescription>
              </DialogHeader>
              <div className="h-[calc(85vh-8rem)] flex items-center justify-center">
                <img
                  src={selectedFigmaImage.url}
                  alt={selectedFigmaImage.nodeName}
                  className="max-h-full max-w-full rounded-lg object-contain"
                />
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
