import { generateComponentList } from '@moatless/types'
import { Copy, Plus, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Separator } from '@/lib/components/ui/separator'
import { cn } from '@/lib/utils'
import type { ElementInfo } from '../types'

interface InspectorPanelProps {
  element: ElementInfo
  className?: string
  onClose: () => void
  onAddToContext?: (element: ElementInfo) => void
}

export function InspectorPanel({
  element,
  className,
  onClose,
  onAddToContext,
}: InspectorPanelProps) {
  const handleCopyLocation = () => {
    const componentList = generateComponentList(element)
    navigator.clipboard
      .writeText(componentList)
      .then(() => {
        // Successfully copied
      })
      .catch((_err) => {
        // Failed to copy - non-critical operation
      })
  }

  const handleClose = () => {
    onClose()
  }

  const handleAddToContext = () => {
    if (onAddToContext) {
      onAddToContext(element)
    }
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
          {/* Prompt */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-muted-foreground">Prompt</span>
              <div className="flex gap-1">
                {onAddToContext && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleAddToContext}
                    className="h-6 px-2"
                  >
                    <Plus className="h-3 w-3 mr-1" />
                    Add to Context
                  </Button>
                )}
                <Button variant="ghost" size="sm" onClick={handleCopyLocation} className="h-6 px-2">
                  <Copy className="h-3 w-3 mr-1" />
                  Copy
                </Button>
              </div>
            </div>
            <pre className="block p-2 bg-muted rounded text-xs whitespace-pre-wrap break-all">
              {generateComponentList(element)}
            </pre>
          </div>

          <Separator />

          {/* Element Info */}
          <div className="space-y-2">
            <h4 className="text-xs font-semibold">Element Details</h4>
            <div className="space-y-1">
              <div className="flex justify-between text-xs">
                <span className="text-muted-foreground">Element</span>
                <span className="font-mono">
                  &lt;{element.componentName || element.elementType || 'unknown'}&gt;
                </span>
              </div>
              <div className="flex justify-between text-xs">
                <span className="text-muted-foreground">File</span>
                <span className="font-mono truncate">{element.file}</span>
              </div>
              <div className="flex justify-between text-xs">
                <span className="text-muted-foreground">Line</span>
                <span className="font-mono">{element.lineNumber}</span>
              </div>
              {element.sourceId && (
                <div className="flex justify-between text-xs">
                  <span className="text-muted-foreground">Source ID</span>
                  <span className="font-mono text-xs truncate">{element.sourceId}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      </ScrollArea>
    </div>
  )
}
