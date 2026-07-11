import { AlertCircle, FileIcon } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { getErrorMessage } from '@/lib/utils/error'
import type { FileAutocompleteItem } from './types'

interface FileAutocompleteProps {
  results: FileAutocompleteItem[]
  selectedIndex: number
  onSelect: (file: FileAutocompleteItem) => void
  isLoading: boolean
  error?: Error | null | undefined
  position?: 'above' | 'below' | undefined
}

export function FileAutocomplete({
  results,
  selectedIndex,
  onSelect,
  isLoading,
  error,
  position = 'below',
}: FileAutocompleteProps) {
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

  if (isLoading) {
    return (
      <div
        data-testid="file-autocomplete"
        className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg p-2`}
      >
        <div className="text-sm text-muted-foreground">Searching files...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div
        data-testid="file-autocomplete"
        className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg p-2`}
      >
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2 text-sm text-status-error">
            <AlertCircle className="h-4 w-4 flex-shrink-0" />
            <span className="font-medium">File search failed</span>
          </div>
          <div className="text-xs text-muted-foreground pl-6">{getErrorMessage(error)}</div>
        </div>
      </div>
    )
  }

  if (results.length === 0) {
    return (
      <div
        data-testid="file-autocomplete"
        className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg p-2`}
      >
        <div className="text-sm text-muted-foreground">No files found</div>
      </div>
    )
  }

  return (
    <div
      ref={listRef}
      data-testid="file-autocomplete"
      className={`absolute z-50 ${positionClasses} max-h-60 w-full overflow-auto rounded-md bg-background border shadow-lg`}
    >
      {results.map((file, index) => {
        const isSelected = index === selectedIndex
        return (
          <button
            key={file.path}
            type="button"
            className={`w-full flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors text-left ${
              isSelected ? 'bg-accent' : 'hover:bg-accent/50'
            }`}
            onClick={() => onSelect(file)}
          >
            <FileIcon className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <div className="flex-1 min-w-0">
              <div className="text-sm truncate">{file.path}</div>
            </div>
          </button>
        )
      })}
    </div>
  )
}
