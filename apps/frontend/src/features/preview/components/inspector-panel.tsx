import { Copy, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Separator } from '@/lib/components/ui/separator'
import { cn } from '@/lib/utils'
import type { ElementInfo } from '../types'
import { generateComponentList } from '../utils/inspector-utils'

interface InspectorPanelProps {
  element: ElementInfo
  className?: string
  onClose: () => void
}

export function InspectorPanel({ element, className, onClose }: InspectorPanelProps) {
  const handleCopyLocation = () => {
    const componentList = generateComponentList(element)
    navigator.clipboard
      .writeText(componentList)
      .then(() => {
        console.log('Component list copied')
      })
      .catch((err) => {
        console.warn('Failed to copy:', err)
      })
  }

  const handleClose = () => {
    onClose()
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
          {/* Relevant Components */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-muted-foreground">Relevant Components</span>
              <Button variant="ghost" size="sm" onClick={handleCopyLocation} className="h-6 px-2">
                <Copy className="h-3 w-3 mr-1" />
                Copy
              </Button>
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
                <span className="text-muted-foreground">Tag</span>
                <span className="font-mono">&lt;{element.elementTag}&gt;</span>
              </div>
              {element.filePath && (
                <>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">Line</span>
                    <span className="font-mono">{element.lineNumber || 'N/A'}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">Column</span>
                    <span className="font-mono">{element.columnNumber || 'N/A'}</span>
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      </ScrollArea>
    </div>
  )
}
