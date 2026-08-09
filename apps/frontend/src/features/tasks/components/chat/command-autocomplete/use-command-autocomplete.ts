import type { SkillResponse } from '@moatless/api-client'
import type React from 'react'
import { useRef, useState } from 'react'

/**
 * Entry in the "/" autocomplete dropdown. Skills come from `.claude/skills/` and
 * `.agents/skills/` in workspace + plugin repos. Picking one activates it as a
 * chip on the message rather than writing anything into the draft: the name
 * travels as a structured field, and the sandbox frontloads the skill's body.
 */
export type Suggestion = { kind: 'skill' } & SkillResponse

interface UseCommandAutocompleteOptions {
  /** Current draft text — the "/term" the dropdown was opened with is stripped on select. */
  value: string
  onValueChange: (value: string) => void
  onSelectSkill: (name: string) => void
  suggestions: Suggestion[]
  enabled?: boolean
}

interface CommandAutocompleteState {
  isOpen: boolean
  searchTerm: string
  selectedIndex: number
}

/** The draft with its leading "/term" trigger removed. */
function stripTrigger(value: string): string {
  if (!value.startsWith('/')) {
    return value
  }
  const spaceIndex = value.indexOf(' ')
  return spaceIndex === -1 ? '' : value.slice(spaceIndex + 1)
}

export function useCommandAutocomplete({
  value,
  onValueChange,
  onSelectSkill,
  suggestions,
  enabled = true,
}: UseCommandAutocompleteOptions) {
  const [state, setState] = useState<CommandAutocompleteState>({
    isOpen: false,
    searchTerm: '',
    selectedIndex: 0,
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
      if (state.isOpen) {
        setState({ isOpen: false, searchTerm: '', selectedIndex: 0 })
      }
      return
    }

    // Extract search term: text between `/` and first space
    const spaceIndex = newValue.indexOf(' ')

    // Past the first space the user is writing the message, not naming a skill.
    if (spaceIndex !== -1) {
      setState({ isOpen: false, searchTerm: '', selectedIndex: 0 })
      return
    }

    const searchTerm = newValue.slice(1)
    setState((prev) => ({
      isOpen: true,
      searchTerm,
      selectedIndex: prev.searchTerm === searchTerm ? prev.selectedIndex : 0,
    }))
  }

  const selectSuggestion = (suggestion: Suggestion) => {
    onSelectSkill(suggestion.name)
    const remaining = stripTrigger(value)
    onValueChange(remaining)

    setState({ isOpen: false, searchTerm: '', selectedIndex: 0 })

    // Move cursor to end
    setTimeout(() => {
      if (inputRef.current) {
        const pos = remaining.length
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

  return {
    inputRef,
    isOpen: state.isOpen,
    filteredCommands: filteredSuggestions,
    selectedIndex: state.selectedIndex,
    handleInputChange,
    handleKeyDown,
    selectCommand: selectSuggestion,
  }
}
