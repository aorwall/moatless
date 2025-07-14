import type { FileSearchResult } from '@moatless/types/types/repository'
import { useCallback, useRef, useState } from 'react'
import { useSearchFiles } from '@/features/repositories/hooks/use-repositories'

interface UseUniversalFileAutocompleteOptions {
  repositoryId?: string
  value: string
  onValueChange: (value: string) => void
  onSelect?: (filePath: string) => void
}

interface AutocompleteState {
  isOpen: boolean
  searchTerm: string
  cursorPosition: number
  selectedIndex: number
}

export function useUniversalFileAutocomplete({
  repositoryId,
  value,
  onValueChange,
  onSelect,
}: UseUniversalFileAutocompleteOptions) {
  const [state, setState] = useState<AutocompleteState>({
    isOpen: false,
    searchTerm: '',
    cursorPosition: 0,
    selectedIndex: 0,
  })

  const inputRef = useRef<HTMLTextAreaElement>(null)

  // Use repository search
  const {
    data: searchResults,
    isLoading,
    error,
  } = useSearchFiles(repositoryId || '', state.searchTerm, {
    enabled: state.isOpen && !!repositoryId && state.searchTerm.length >= 3,
  })

  const handleInputChange = useCallback((value: string, cursorPosition: number) => {
    // Find the @ symbol before the cursor
    const textBeforeCursor = value.substring(0, cursorPosition)
    const lastAtIndex = textBeforeCursor.lastIndexOf('@')

    if (lastAtIndex === -1) {
      setState((prev) => ({ ...prev, isOpen: false, searchTerm: '', selectedIndex: 0 }))
      return
    }

    // Check if we're in a valid @ context
    const textAfterAt = value.substring(lastAtIndex + 1, cursorPosition)
    const spaceIndex = textAfterAt.indexOf(' ')

    if (spaceIndex !== -1) {
      // There's a space after @, so we're not in autocomplete mode
      setState((prev) => ({ ...prev, isOpen: false, searchTerm: '', selectedIndex: 0 }))
      return
    }

    // Extract the search term
    const searchTerm = textAfterAt

    setState((prev) => ({
      isOpen: true,
      searchTerm,
      cursorPosition: lastAtIndex,
      // Reset selectedIndex when search term changes
      selectedIndex: prev.searchTerm !== searchTerm ? 0 : prev.selectedIndex,
    }))
  }, [])

  const selectFile = useCallback(
    (file: FileSearchResult) => {
      const { cursorPosition, searchTerm } = state

      // Replace @searchTerm with @filepath
      const before = value.substring(0, cursorPosition)
      const after = value.substring(cursorPosition + searchTerm.length + 1)
      const newValue = `${before}@${file.path}${after}`

      // Update the value through parent
      onValueChange(newValue)

      // Close autocomplete
      setState({
        isOpen: false,
        searchTerm: '',
        cursorPosition: 0,
        selectedIndex: 0,
      })

      // Schedule cursor position update
      setTimeout(() => {
        if (inputRef.current) {
          const newCursorPosition = cursorPosition + file.path.length + 1
          inputRef.current.setSelectionRange(newCursorPosition, newCursorPosition)
          inputRef.current.focus()
        }
      }, 0)

      // Call onSelect callback if provided
      onSelect?.(file.path)
    },
    [state, value, onValueChange, onSelect],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!(state.isOpen && searchResults?.results.length)) {
        return
      }

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setState((prev) => ({
            ...prev,
            selectedIndex: Math.min(prev.selectedIndex + 1, searchResults.results.length - 1),
          }))
          break
        case 'ArrowUp':
          e.preventDefault()
          setState((prev) => ({
            ...prev,
            selectedIndex: Math.max(prev.selectedIndex - 1, 0),
          }))
          break
        case 'Enter':
          if (state.isOpen && searchResults.results[state.selectedIndex]) {
            e.preventDefault()
            selectFile(searchResults.results[state.selectedIndex])
          }
          break
        case 'Escape':
          e.preventDefault()
          e.stopPropagation()
          setState((prev) => ({ ...prev, isOpen: false }))
          break
        case 'Tab':
          if (state.isOpen && searchResults.results[state.selectedIndex]) {
            e.preventDefault()
            selectFile(searchResults.results[state.selectedIndex])
          }
          break
        default:
          break
      }
    },
    [state.isOpen, state.selectedIndex, searchResults, selectFile],
  )

  return {
    inputRef,
    isOpen: state.isOpen,
    searchTerm: state.searchTerm,
    selectedIndex: state.selectedIndex,
    results: searchResults?.results || [],
    isLoading,
    error,
    handleInputChange,
    handleKeyDown,
    selectFile,
  }
}
