import { Code2, Sparkles } from 'lucide-react'
import type { ChatInputMode } from '@/features/tasks/components/chat-input'
import { ToggleGroup, ToggleGroupItem } from '@/lib/components/ui/toggle-group'

interface ViewToggleProps {
  mode: ChatInputMode
  onModeChange: (mode: ChatInputMode) => void
  className?: string
}

export function ViewToggle({ mode, onModeChange, className }: ViewToggleProps) {
  return (
    <ToggleGroup
      type="single"
      value={mode}
      onValueChange={(value: string) => value && onModeChange(value as ChatInputMode)}
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