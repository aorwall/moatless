import { Sparkles } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { getCommandIcon } from '@/features/tasks/components/chat-view/command-icons'
import type { Suggestion } from './use-command-autocomplete'

interface CommandAutocompleteProps {
  commands: Suggestion[]
  selectedIndex: number
  onSelect: (suggestion: Suggestion) => void
  position?: 'above' | 'below'
}

function suggestionKey(suggestion: Suggestion): string {
  return `${suggestion.kind}:${suggestion.name}`
}

export function CommandAutocomplete({
  commands,
  selectedIndex,
  onSelect,
  position = 'above',
}: CommandAutocompleteProps) {
  const listRef = useRef<HTMLDivElement>(null)

  // Scroll selected item into view
  useEffect(() => {
    if (listRef.current) {
      const selectedElement = listRef.current.children[selectedIndex] as HTMLElement
      if (selectedElement) {
        selectedElement.scrollIntoView({ block: 'nearest' })
      }
    }
  }, [selectedIndex])

  const positionClasses = position === 'above' ? 'bottom-full mb-1' : 'top-full mt-1'

  if (commands.length === 0) {
    return (
      <div
        data-testid="command-autocomplete"
        className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg p-2`}
      >
        <div className="text-sm text-muted-foreground">No commands found</div>
      </div>
    )
  }

  return (
    <div
      ref={listRef}
      data-testid="command-autocomplete"
      className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg`}
    >
      {commands.map((suggestion, index) => {
        const isSelected = index === selectedIndex
        const Icon =
          suggestion.kind === 'command' ? getCommandIcon(suggestion.icon ?? undefined) : Sparkles
        const badge =
          suggestion.kind === 'skill'
            ? suggestion.source === 'plugin' && suggestion.pluginRepoName
              ? suggestion.pluginRepoName
              : 'skill'
            : null
        return (
          <button
            key={suggestionKey(suggestion)}
            type="button"
            className={`w-full flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors text-left ${
              isSelected ? 'bg-accent' : 'hover:bg-accent/50'
            }`}
            onClick={() => onSelect(suggestion)}
          >
            <Icon className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <div className="flex-1 min-w-0 flex items-baseline gap-2">
              <span className="text-sm font-medium flex-shrink-0">/{suggestion.name}</span>
              <span className="text-sm text-muted-foreground truncate">
                {suggestion.description}
              </span>
            </div>
            {badge && (
              <span className="text-[10px] uppercase tracking-wide text-muted-foreground bg-muted px-1.5 py-0.5 rounded flex-shrink-0">
                {badge}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
