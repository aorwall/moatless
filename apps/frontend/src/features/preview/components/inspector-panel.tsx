import { Plus, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import type { ElementInfo } from '../types'

interface InspectorPanelProps {
  element: ElementInfo
  className?: string
  onClose: () => void
  onAddToContext?: (element: ElementInfo) => void
}

interface Position {
  top: number
  left: number
}

export function InspectorPanel({
  element,
  className,
  onClose,
  onAddToContext,
}: InspectorPanelProps) {
  const [position, setPosition] = useState<Position>({ top: 0, left: 0 })
  const POPOVER_WIDTH = 200
  const POPOVER_HEIGHT = 100 // Approximate height
  const OFFSET = 12 // Gap between element and popover

  useEffect(() => {
    if (!element.elementRect) {
      // Fallback to center if no position data
      setPosition({
        top: window.innerHeight / 2 - POPOVER_HEIGHT / 2,
        left: window.innerWidth / 2 - POPOVER_WIDTH / 2,
      })
      return
    }

    const rect = element.elementRect
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight

    let top = rect.bottom + OFFSET
    let left = rect.left + rect.width / 2 - POPOVER_WIDTH / 2

    // If too close to bottom, show above element
    if (top + POPOVER_HEIGHT > viewportHeight - OFFSET) {
      top = rect.top - POPOVER_HEIGHT - OFFSET
    }

    // If too close to right edge, align right
    if (left + POPOVER_WIDTH > viewportWidth - OFFSET) {
      left = viewportWidth - POPOVER_WIDTH - OFFSET
    }

    // If too close to left edge, align left
    if (left < OFFSET) {
      left = OFFSET
    }

    // Ensure top is not negative
    if (top < OFFSET) {
      top = OFFSET
    }

    setPosition({ top, left })
  }, [element.elementRect])

  const handleAddToContext = () => {
    if (onAddToContext) {
      onAddToContext(element)
    }
  }

  const elementName = element.componentName || element.elementType || 'unknown'

  return (
    <div
      className={cn('fixed z-50 w-[200px] bg-background border rounded-lg shadow-lg', className)}
      style={{ top: `${position.top}px`, left: `${position.left}px` }}
    >
      <div className="p-2 space-y-2">
        <div className="flex items-center justify-between gap-1">
          <span className="text-xs font-semibold truncate">&lt;{elementName}&gt;</span>
          <Button variant="ghost" size="icon" onClick={onClose} className="h-5 w-5 flex-shrink-0">
            <X className="h-3 w-3" />
          </Button>
        </div>

        {onAddToContext && (
          <Button
            variant="default"
            size="sm"
            onClick={handleAddToContext}
            className="h-7 w-full text-xs"
          >
            <Plus className="h-3 w-3 mr-1" />
            Add to message
          </Button>
        )}
      </div>
    </div>
  )
}
