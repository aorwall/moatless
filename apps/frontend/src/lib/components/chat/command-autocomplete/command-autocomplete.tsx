import type { CommandResponse } from '@moatless/api-client'
import { useEffect, useRef } from 'react'
import { getCommandIcon } from '@/features/tasks/components/chat-view/command-buttons'

interface CommandAutocompleteProps {
  commands: CommandResponse[]
  selectedIndex: number
  onSelect: (command: CommandResponse) => void
  position?: 'above' | 'below'
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
      {commands.map((command, index) => {
        const isSelected = index === selectedIndex
        const Icon = getCommandIcon(command.icon ?? undefined)
        return (
          <button
            key={command.name}
            type="button"
            className={`w-full flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors text-left ${
              isSelected ? 'bg-accent' : 'hover:bg-accent/50'
            }`}
            onClick={() => onSelect(command)}
          >
            <Icon className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <div className="flex-1 min-w-0">
              <span className="text-sm font-medium">/{command.name}</span>
              <span className="text-sm text-muted-foreground ml-2">{command.description}</span>
            </div>
          </button>
        )
      })}
    </div>
  )
}
