import type { MessageContext } from '@moatless/types'
import { Eye, X } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

interface ContextItemsProps {
  contexts: MessageContext[]
  onRemove?: (index: number) => void
}

interface ElementDetailPopoverProps {
  element: MessageContext['data']
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

  if (contexts.length === 0) {
    return null
  }

  return (
    <div className="flex flex-col gap-2 mb-2">
      <div className="flex flex-wrap gap-2">
        {contexts.map((context, index) => {
          if (context.type === 'element') {
            const element = context.data
            const displayName = element.componentName || element.elementType || 'Unknown Element'
            // Create a unique key from element properties
            const uniqueKey = element.sourceId || `${element.path}-${element.lineNumber}-${index}`

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
                        onRemove(index)
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
    </div>
  )
}
