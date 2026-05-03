import { ChevronDown, ChevronRight } from 'lucide-react'
import { type ReactNode, useState } from 'react'

interface ToolCallGroupProps {
  count: number
  children: ReactNode
}

export function ToolCallGroup({ count, children }: ToolCallGroupProps) {
  const [expanded, setExpanded] = useState(false)
  const label = count === 1 ? 'Ran 1 command' : `Ran ${count} commands`

  return (
    <div className="border border-dashed border-muted-foreground/30 rounded-lg overflow-hidden bg-muted/10">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 p-2 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
      >
        {expanded ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0" />
        )}
        <span className="font-medium tabular-nums">{label}</span>
      </button>
      {expanded && (
        <div className="p-3 border-t border-dashed border-muted-foreground/20">{children}</div>
      )}
    </div>
  )
}
