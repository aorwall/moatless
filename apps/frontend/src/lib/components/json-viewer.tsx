import { Check, ChevronDown, ChevronRight, Copy } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { allExpanded, collapseAllNested, JsonView } from 'react-json-view-lite'
import 'react-json-view-lite/dist/index.css'
import { toast } from 'sonner'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'

interface JsonViewerProps {
  data: unknown
  className?: string
  maxHeight?: string
  onPrimitiveClick?: (value: string | number) => void
  enableClickableValues?: boolean
}

/**
 * JSON Viewer Component
 * Displays JSON data with collapsible tree view and copy functionality
 * Optimized for large JSON datasets using react-json-view-lite
 */
export function JsonViewer({
  data,
  className = '',
  maxHeight = '100%',
  onPrimitiveClick,
  enableClickableValues = false,
}: JsonViewerProps) {
  const [copied, setCopied] = useState(false)
  const [expandAll, setExpandAll] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)

  const jsonString = JSON.stringify(data, null, 2)

  // Add click event listener for primitive value clicks
  useEffect(() => {
    if (!(enableClickableValues && onPrimitiveClick)) {
      return
    }

    const container = containerRef.current
    if (!container) {
      return
    }

    const handleClick = (event: MouseEvent) => {
      const target = event.target as HTMLElement

      // Debug: log clicked element
      // biome-ignore lint/suspicious/noConsole: Temporary debugging for click functionality
      console.log('Clicked element:', {
        tagName: target.tagName,
        classList: Array.from(target.classList),
        textContent: target.textContent?.trim(),
      })

      // Only process clicks on span elements
      if (target.tagName !== 'SPAN') {
        return
      }

      const textContent = target.textContent?.trim()
      if (!textContent) {
        return
      }

      // Try to parse the value
      let value: string | number

      // Parse string (remove quotes)
      if (textContent.startsWith('"') && textContent.endsWith('"')) {
        value = textContent.slice(1, -1)
      }
      // Parse number
      else if (!Number.isNaN(Number(textContent))) {
        value = Number(textContent)
      }
      // Skip booleans, null, and other non-primitive values
      else if (['true', 'false', 'null', '{', '}', '[', ']', ',', ':'].includes(textContent)) {
        return
      }
      // If we can't parse it, skip
      else {
        return
      }

      // biome-ignore lint/suspicious/noConsole: Temporary debugging for click functionality
      console.log('Triggering search for:', value)
      onPrimitiveClick(value)
    }

    container.addEventListener('click', handleClick)
    return () => container.removeEventListener('click', handleClick)
  }, [enableClickableValues, onPrimitiveClick])

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(jsonString)
      setCopied(true)
      toast.success('Copied to clipboard')
      setTimeout(() => setCopied(false), 2000)
    } catch (_err) {
      toast.error('Failed to copy to clipboard')
    }
  }

  // Handle null/undefined data gracefully
  if (data === null || data === undefined) {
    return (
      <div className={`rounded-md border border-dashed p-8 text-center ${className}`}>
        <p className="text-sm text-muted-foreground">No data available</p>
      </div>
    )
  }

  return (
    <div className={`relative ${className}`}>
      <div className="absolute right-2 top-2 z-10 flex gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => setExpandAll(!expandAll)}
          className="h-8 gap-2"
        >
          {expandAll ? (
            <>
              <ChevronRight className="h-3 w-3" />
              Collapse All
            </>
          ) : (
            <>
              <ChevronDown className="h-3 w-3" />
              Expand All
            </>
          )}
        </Button>
        <Button size="sm" variant="outline" onClick={handleCopy} className="h-8 gap-2">
          {copied ? (
            <>
              <Check className="h-3 w-3" />
              Copied
            </>
          ) : (
            <>
              <Copy className="h-3 w-3" />
              Copy
            </>
          )}
        </Button>
      </div>
      <div
        ref={containerRef}
        className={cn(
          'overflow-auto rounded-md border bg-background p-4',
          enableClickableValues && 'json-viewer-clickable',
        )}
        style={{ maxHeight }}
      >
        <JsonView data={data} shouldExpandNode={expandAll ? allExpanded : collapseAllNested} />
      </div>
    </div>
  )
}
