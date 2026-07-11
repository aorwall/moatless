import type React from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { FileAutocompleteItem } from './types'

interface UseFileAutocompleteOptions {
  value: string
  onValueChange: (value: string) => void
  onSelect?: ((filePath: string) => void) | undefined
  enabled?: boolean | undefined
  // Search behavior injected from outside
  onSearch: (searchTerm: string) => void
  results: FileAutocompleteItem[]
  isLoading: boolean
  error: Error | null
}

interface AutocompleteState {
  isOpen: boolean
  searchTerm: string
  cursorPosition: number
  selectedIndex: number
}

export function useFileAutocomplete({
  value,
  onValueChange,
  onSelect,
  enabled = true,
  onSearch,
  results,
  isLoading,
  error,
}: UseFileAutocompleteOptions) {
  const [state, setState] = useState<AutocompleteState>({
    isOpen: false,
    searchTerm: '',
    cursorPosition: 0,
    selectedIndex: 0,
  })

  const inputRef = useRef<HTMLTextAreaElement>(null)
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Trigger search when search term changes (debounced)
  // biome-ignore lint/correctness/useExhaustiveDependencies: onSearch is expected to be stable from caller
  useEffect(() => {
    if (!(enabled && state.isOpen && state.searchTerm) || state.searchTerm.length < 2) {
      return
    }

    // Clear existing timer
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current)
    }

    // Set new timer
    debounceTimerRef.current = setTimeout(() => {
      onSearch(state.searchTerm)
    }, 300)

    // Cleanup
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current)
      }
    }
  }, [state.isOpen, state.searchTerm, enabled])

  const handleInputChange = useCallback(
    (value: string, cursorPosition: number) => {
      if (!enabled) {
        return
      }

      // Find the @ symbol before the cursor
      const textBeforeCursor = value.substring(0, cursorPosition)
      const lastAtIndex = textBeforeCursor.lastIndexOf('@')

      if (lastAtIndex === -1) {
        setState((prev) => ({
          ...prev,
          isOpen: false,
          searchTerm: '',
          selectedIndex: 0,
        }))
        return
      }

      // Check if we're in a valid @ context
      const textAfterAt = value.substring(lastAtIndex + 1, cursorPosition)
      const spaceIndex = textAfterAt.indexOf(' ')

      if (spaceIndex !== -1) {
        // There's a space after @, so we're not in autocomplete mode
        setState((prev) => ({
          ...prev,
          isOpen: false,
          searchTerm: '',
          selectedIndex: 0,
        }))
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
    },
    [enabled],
  )

  const selectFile = useCallback(
    (file: FileAutocompleteItem) => {
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
      if (!(state.isOpen && results.length > 0)) {
        return
      }

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setState((prev) => ({
            ...prev,
            selectedIndex: Math.min(prev.selectedIndex + 1, results.length - 1),
          }))
          break
        case 'ArrowUp':
          e.preventDefault()
          setState((prev) => ({
            ...prev,
            selectedIndex: Math.max(prev.selectedIndex - 1, 0),
          }))
          break
        case 'Enter': {
          const enterItem = results[state.selectedIndex]
          if (state.isOpen && enterItem) {
            e.preventDefault()
            selectFile(enterItem)
          }
          break
        }
        case 'Escape':
          e.preventDefault()
          e.stopPropagation()
          setState((prev) => ({ ...prev, isOpen: false }))
          break
        case 'Tab': {
          const tabItem = results[state.selectedIndex]
          if (state.isOpen && tabItem) {
            e.preventDefault()
            selectFile(tabItem)
          }
          break
        }
        default:
          break
      }
    },
    [state.isOpen, state.selectedIndex, results, selectFile],
  )

  return {
    inputRef,
    isOpen: state.isOpen,
    searchTerm: state.searchTerm,
    selectedIndex: state.selectedIndex,
    results,
    isLoading,
    error,
    handleInputChange,
    handleKeyDown,
    selectFile,
  }
}
