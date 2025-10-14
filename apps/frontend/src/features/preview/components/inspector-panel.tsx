import { generateComponentList } from '@moatless/types'
import { Copy, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Separator } from '@/lib/components/ui/separator'
import { cn } from '@/lib/utils'
import type { ElementInfo } from '../types'

interface InspectorPanelProps {
  element: ElementInfo
  className?: string
  onClose: () => void
}

export function InspectorPanel({ element, className, onClose }: InspectorPanelProps) {
  // Parse URL-encoded component props JSON
  const parseComponentContent = (content: string | null): Record<string, unknown> | null => {
    if (!content) {
      return null
    }
    try {
      const decoded = decodeURIComponent(content)
      return JSON.parse(decoded) as Record<string, unknown>
    } catch {
      return null
    }
  }

  const componentProps = parseComponentContent(element.componentContent)

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

  const handleCopyProps = () => {
    if (componentProps) {
      navigator.clipboard
        .writeText(JSON.stringify(componentProps, null, 2))
        .then(() => {
          // Successfully copied
        })
        .catch((_err) => {
          // Failed to copy - non-critical operation
        })
    }
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
          {/* Prompt */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-muted-foreground">Prompt</span>
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
                <span className="text-muted-foreground">Element</span>
                <span className="font-mono">
                  &lt;{element.componentName || element.elementType || 'unknown'}&gt;
                </span>
              </div>
              {element.componentPath && (
                <>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">File</span>
                    <span className="font-mono truncate">{element.componentFile || 'N/A'}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">Line</span>
                    <span className="font-mono">{element.componentLine || 'N/A'}</span>
                  </div>
                </>
              )}
              {element.sourceId && (
                <div className="flex justify-between text-xs">
                  <span className="text-muted-foreground">Source ID</span>
                  <span className="font-mono text-xs truncate">{element.sourceId}</span>
                </div>
              )}
            </div>
          </div>

          {/* Component Props */}
          {componentProps && (
            <>
              <Separator />
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-semibold">Component Props</h4>
                  <Button variant="ghost" size="sm" onClick={handleCopyProps} className="h-6 px-2">
                    <Copy className="h-3 w-3 mr-1" />
                    Copy
                  </Button>
                </div>
                <pre className="block p-2 bg-muted rounded text-xs whitespace-pre-wrap break-all font-mono overflow-x-auto">
                  {JSON.stringify(componentProps, null, 2)}
                </pre>
              </div>
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
