import type { MessageContext } from '@moatless/types'
import { Eye, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

interface ContextItemsProps {
  contexts: MessageContext[]
  onRemove?: (index: number) => void
}

export function ContextItems({ contexts, onRemove }: ContextItemsProps) {
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
              <div
                key={uniqueKey}
                className="flex items-center gap-1.5 px-2 py-1 bg-secondary/10 rounded text-xs group"
              >
                <Eye className="h-3 w-3 text-muted-foreground" />
                <span className="max-w-[150px] truncate" title={displayName}>
                  {displayName}
                </span>
                <span className="text-muted-foreground">
                  in {element.file}:{element.lineNumber}
                </span>
                {onRemove && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() => onRemove(index)}
                    className="h-4 w-4 p-0 text-muted-foreground hover:text-foreground transition-colors opacity-0 group-hover:opacity-100"
                    aria-label="Remove context"
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
    </div>
  )
}
