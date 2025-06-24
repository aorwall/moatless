import { useCallback, useEffect, useRef, useState } from 'react'

import { useSearchFiles } from '@/features/environments/hooks/use-environments'
import { useTaskFileSearch } from '@/features/tasks/hooks/use-tasks'
import type { FileSearchResult } from '@/types/environment'

interface UseUniversalFileAutocompleteOptions {
  environmentId?: string
  taskId?: string
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
  environmentId,
  taskId,
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

  // Use environment search when in environment context
  const {
    data: envSearchResults,
    isLoading: envLoading,
    error: envError,
  } = useSearchFiles(environmentId || '', state.searchTerm, {
    enabled: state.isOpen && !!environmentId && !taskId && state.searchTerm.length >= 3,
  })

  // Use task search when in task context
  const {
    data: taskSearchResults,
    isLoading: taskLoading,
    error: taskError,
  } = useTaskFileSearch(taskId || '', state.searchTerm, {
    enabled: state.isOpen && !!taskId && state.searchTerm.length >= 3,
  })

  // Determine which results to use
  const searchResults = taskId ? taskSearchResults : envSearchResults
  const isLoading = taskId ? taskLoading : envLoading
  const error = taskId ? taskError : envError

  const handleInputChange = useCallback((value: string, cursorPosition: number) => {
    // Find the @ symbol before the cursor
    const textBeforeCursor = value.substring(0, cursorPosition)
    const lastAtIndex = textBeforeCursor.lastIndexOf('@')

    if (lastAtIndex === -1) {
      setState((prev) => ({ ...prev, isOpen: false, searchTerm: '' }))
      return
    }

    // Check if we're in a valid @ context
    const textAfterAt = value.substring(lastAtIndex + 1, cursorPosition)
    const spaceIndex = textAfterAt.indexOf(' ')

    if (spaceIndex !== -1) {
      // There's a space after @, so we're not in autocomplete mode
      setState((prev) => ({ ...prev, isOpen: false, searchTerm: '' }))
      return
    }

    // Extract the search term
    const searchTerm = textAfterAt

    setState({
      isOpen: true,
      searchTerm,
      cursorPosition: lastAtIndex,
      selectedIndex: 0,
    })
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

  // Reset selected index when results change
  useEffect(() => {
    setState((prev) => ({ ...prev, selectedIndex: 0 }))
  }, [])

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
