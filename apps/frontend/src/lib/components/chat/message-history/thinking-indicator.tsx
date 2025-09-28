import { Lightbulb } from 'lucide-react'

export function ThinkingIndicator() {
  return (
    <div className="flex items-center gap-2 text-muted-foreground animate-pulse">
      <Lightbulb className="h-4 w-4" />
      <span className="text-sm">Thinking</span>
    </div>
  )
}
