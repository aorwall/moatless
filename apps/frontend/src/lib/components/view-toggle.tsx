import { Code2, Sparkles } from 'lucide-react'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group.tsx'

type ViewMode = 'code' | 'vibe'

interface ViewToggleProps {
  mode: ViewMode
  onModeChange: (mode: ViewMode) => void
  className?: string
}

export function ViewToggle({ mode, onModeChange, className }: ViewToggleProps) {
  return (
    <ToggleGroup
      type="single"
      value={mode}
      onValueChange={(value: string) => value && onModeChange(value as ViewMode)}
      className={className}
    >
      <ToggleGroupItem value="code" aria-label="Code mode" size="sm">
        <Code2 className="h-4 w-4 mr-2" />
        Code
      </ToggleGroupItem>
      <ToggleGroupItem value="vibe" aria-label="Vibe mode" size="sm">
        <Sparkles className="h-4 w-4 mr-2" />
        Vibe
      </ToggleGroupItem>
    </ToggleGroup>
  )
}
