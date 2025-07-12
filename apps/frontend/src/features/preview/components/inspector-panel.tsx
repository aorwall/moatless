import { Copy, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Separator } from '@/lib/components/ui/separator'
import { cn } from '@/lib/utils'
import { usePreview } from '../hooks/use-preview'

interface InspectorPanelProps {
  className?: string
  onClose?: () => void
}

export function InspectorPanel({ className, onClose }: InspectorPanelProps) {
  const { selectedElement, setSelectedElement } = usePreview()

  if (!selectedElement) {
    return null
  }

  const handleCopySelector = () => {
    if (selectedElement.displayText) {
      navigator.clipboard
        .writeText(selectedElement.displayText)
        .then(() => {
          console.log('Selector copied:', selectedElement.displayText)
        })
        .catch((err) => {
          console.warn('Failed to copy:', err)
        })
    }
  }

  const handleClose = () => {
    setSelectedElement(null)
    onClose?.()
  }

  return (
    <div
      className={cn(
        'absolute bottom-4 right-4 w-80 bg-background border rounded-lg shadow-lg',
        className,
      )}
    >
      <div className="flex items-center justify-between p-3 border-b">
        <h3 className="text-sm font-semibold">Element Inspector</h3>
        <Button variant="ghost" size="icon" onClick={handleClose} className="h-6 w-6">
          <X className="h-4 w-4" />
        </Button>
      </div>

      <ScrollArea className="h-80">
        <div className="p-3 space-y-3">
          {/* Selector */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-muted-foreground">Selector</span>
              <Button variant="ghost" size="sm" onClick={handleCopySelector} className="h-6 px-2">
                <Copy className="h-3 w-3 mr-1" />
                Copy
              </Button>
            </div>
            <code className="block p-2 bg-muted rounded text-xs break-all">
              {selectedElement.displayText}
            </code>
          </div>

          <Separator />

          {/* Element Info */}
          <div className="space-y-2">
            <h4 className="text-xs font-semibold">Element Details</h4>

            <div className="space-y-1">
              <div className="flex justify-between text-xs">
                <span className="text-muted-foreground">Tag</span>
                <span className="font-mono">{selectedElement.tagName}</span>
              </div>

              {selectedElement.id && (
                <div className="flex justify-between text-xs">
                  <span className="text-muted-foreground">ID</span>
                  <span className="font-mono">#{selectedElement.id}</span>
                </div>
              )}

              {selectedElement.className && (
                <div className="text-xs">
                  <span className="text-muted-foreground">Classes</span>
                  <div className="mt-1 space-y-1">
                    {selectedElement.className
                      .split(' ')
                      .filter((c) => c)
                      .map((cls) => (
                        <code key={cls} className="block p-1 bg-muted rounded text-xs">
                          .{cls}
                        </code>
                      ))}
                  </div>
                </div>
              )}
            </div>
          </div>

          <Separator />

          {/* Computed Styles */}
          <div className="space-y-2">
            <h4 className="text-xs font-semibold">Computed Styles</h4>
            <div className="space-y-1">
              {Object.entries(selectedElement.computedStyles).map(([key, value]) => (
                <div key={key} className="flex justify-between text-xs">
                  <span className="text-muted-foreground">{key}</span>
                  <span className="font-mono truncate max-w-[180px]">{value}</span>
                </div>
              ))}
            </div>
          </div>

          <Separator />

          {/* Dimensions */}
          <div className="space-y-2">
            <h4 className="text-xs font-semibold">Dimensions</h4>
            <div className="grid grid-cols-2 gap-2 text-xs">
              <div>
                <span className="text-muted-foreground">Width:</span>
                <span className="ml-1 font-mono">
                  {Math.round(selectedElement.boundingRect.width)}px
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">Height:</span>
                <span className="ml-1 font-mono">
                  {Math.round(selectedElement.boundingRect.height)}px
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">X:</span>
                <span className="ml-1 font-mono">
                  {Math.round(selectedElement.boundingRect.x)}px
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">Y:</span>
                <span className="ml-1 font-mono">
                  {Math.round(selectedElement.boundingRect.y)}px
                </span>
              </div>
            </div>
          </div>

          {/* Text Content */}
          {selectedElement.textContent && (
            <>
              <Separator />
              <div className="space-y-2">
                <h4 className="text-xs font-semibold">Text Content</h4>
                <p className="text-xs text-muted-foreground">{selectedElement.textContent}</p>
              </div>
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
