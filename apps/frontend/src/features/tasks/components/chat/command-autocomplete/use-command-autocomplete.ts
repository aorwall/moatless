import type { SkillResponse } from '@moatless/api-client'
import type React from 'react'
import { useRef, useState } from 'react'

/**
 * Entry in the "/" autocomplete dropdown. Skills come from `.claude/skills/` in
 * workspace + plugin repos and are passed through verbatim to the sandbox's
 * Claude Code CLI (invoked by typing `/skill` in the message).
 */
export type Suggestion = { kind: 'skill' } & SkillResponse

interface UseCommandAutocompleteOptions {
  onValueChange: (value: string) => void
  suggestions: Suggestion[]
  enabled?: boolean
}

interface SelectionKey {
  kind: 'skill'
  name: string
}

interface CommandAutocompleteState {
  isOpen: boolean
  searchTerm: string
  selectedIndex: number
  /** The suggestion picked from the dropdown (command or skill). */
  selection: SelectionKey | null
}

function suggestionKey(s: Suggestion): string {
  return `${s.kind}:${s.name}`
}

export function useCommandAutocomplete({
  onValueChange,
  suggestions,
  enabled = true,
}: UseCommandAutocompleteOptions) {
  const [state, setState] = useState<CommandAutocompleteState>({
    isOpen: false,
    searchTerm: '',
    selectedIndex: 0,
    selection: null,
  })

  const inputRef = useRef<HTMLTextAreaElement>(null)

  // Filter suggestions client-side based on the current search term
  const filteredSuggestions = (() => {
    if (!state.isOpen || suggestions.length === 0) {
      return []
    }
    if (!state.searchTerm) {
      return suggestions
    }
    const term = state.searchTerm.toLowerCase()
    return suggestions.filter(
      (s) => s.name.toLowerCase().includes(term) || s.description.toLowerCase().includes(term),
    )
  })()

  const handleInputChange = (newValue: string, _cursorPosition: number) => {
    if (!enabled || suggestions.length === 0) {
      return
    }

    // Only activate when `/` is the very first character
    if (newValue.length === 0 || newValue[0] !== '/') {
      if (state.selection) {
        setState((prev) => ({ ...prev, selection: null, isOpen: false, searchTerm: '' }))
      } else if (state.isOpen) {
        setState((prev) => ({ ...prev, isOpen: false, searchTerm: '', selectedIndex: 0 }))
      }
      return
    }

    // If something was already selected, check if the prefix still matches
    if (state.selection) {
      const expectedPrefix = `/${state.selection.name} `
      if (!newValue.startsWith(expectedPrefix)) {
        // User edited the command portion — clear selection and re-open autocomplete
        setState((prev) => ({ ...prev, selection: null }))
        // Fall through to re-parse
      } else {
        // Prefix still intact, user is typing after the command — no autocomplete needed
        return
      }
    }

    // Extract search term: text between `/` and first space
    const spaceIndex = newValue.indexOf(' ')
    const searchTerm = spaceIndex === -1 ? newValue.slice(1) : newValue.slice(1, spaceIndex)

    // If there's a space, the user may have finished typing the command portion
    // but didn't select from dropdown — keep autocomplete closed
    if (spaceIndex !== -1) {
      setState((prev) => ({ ...prev, isOpen: false, searchTerm: '', selectedIndex: 0 }))
      return
    }

    setState((prev) => ({
      isOpen: true,
      searchTerm,
      selection: prev.selection,
      selectedIndex: prev.searchTerm !== searchTerm ? 0 : prev.selectedIndex,
    }))
  }

  const selectSuggestion = (suggestion: Suggestion) => {
    const newValue = `/${suggestion.name} `
    onValueChange(newValue)

    setState({
      isOpen: false,
      searchTerm: '',
      selectedIndex: 0,
      selection: { kind: suggestion.kind, name: suggestion.name },
    })

    // Move cursor to end
    setTimeout(() => {
      if (inputRef.current) {
        const pos = newValue.length
        inputRef.current.setSelectionRange(pos, pos)
        inputRef.current.focus()
      }
    }, 0)
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (!(state.isOpen && filteredSuggestions.length > 0)) {
      return
    }

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        setState((prev) => ({
          ...prev,
          selectedIndex: Math.min(prev.selectedIndex + 1, filteredSuggestions.length - 1),
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
        const enterSuggestion = filteredSuggestions[state.selectedIndex]
        if (enterSuggestion) {
          e.preventDefault()
          selectSuggestion(enterSuggestion)
        }
        break
      }
      case 'Tab': {
        const tabSuggestion = filteredSuggestions[state.selectedIndex]
        if (tabSuggestion) {
          e.preventDefault()
          selectSuggestion(tabSuggestion)
        }
        break
      }
      case 'Escape':
        e.preventDefault()
        e.stopPropagation()
        setState((prev) => ({ ...prev, isOpen: false }))
        break
      default:
        break
    }
  }

  /** The full suggestion record for the currently selected item, if any. */
  const selectedItem = (() => {
    if (!state.selection) {
      return null
    }
    const key = `${state.selection.kind}:${state.selection.name}`
    return suggestions.find((s) => suggestionKey(s) === key) ?? null
  })()

  return {
    inputRef,
    isOpen: state.isOpen,
    filteredCommands: filteredSuggestions,
    selectedIndex: state.selectedIndex,
    selectedItem,
    handleInputChange,
    handleKeyDown,
    selectCommand: selectSuggestion,
  }
}
