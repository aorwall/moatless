import { AlertTriangle, Brain, ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { Response } from '@/lib/components/ai-elements/response'

/**
 * Red card for non-fatal codex error items so the user notices them; the
 * turn itself is still running or completed normally.
 */
export function CodexErrorCard({ text }: { text: string }) {
  return (
    <div className="border border-red-500/40 rounded-lg bg-red-500/5 p-3 flex items-start gap-2">
      <AlertTriangle className="h-4 w-4 flex-shrink-0 text-red-500 mt-0.5" />
      <div className="text-sm text-red-600 dark:text-red-400 whitespace-pre-wrap break-words">
        {text}
      </div>
    </div>
  )
}

/**
 * Collapsible card for codex reasoning messages, rendered instead of inline
 * assistant text.
 */
export function CodexReasoningCard({ text }: { text: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="border border-dashed border-muted-foreground/30 rounded-lg overflow-hidden bg-muted/10">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full flex items-center gap-2 p-2 text-xs text-muted-foreground hover:bg-muted/30 transition-colors"
      >
        {open ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0" />
        )}
        <Brain className="h-3 w-3 flex-shrink-0" />
        <span className="font-medium">Reasoning</span>
        {!open && (
          <span className="truncate text-muted-foreground/70">
            {text.slice(0, 80)}
            {text.length > 80 ? '...' : ''}
          </span>
        )}
      </button>
      {open && (
        <div className="p-3 border-t border-dashed border-muted-foreground/20 text-sm">
          <Response>{text}</Response>
        </div>
      )}
    </div>
  )
}
