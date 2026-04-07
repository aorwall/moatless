import type { CommandResponse } from '@moatless/api-client'
import type React from 'react'
import { useCallback, useMemo, useRef, useState } from 'react'

interface UseCommandAutocompleteOptions {
  onValueChange: (value: string) => void
  commands: CommandResponse[]
  enabled?: boolean
}

interface CommandAutocompleteState {
  isOpen: boolean
  searchTerm: string
  selectedIndex: number
  /** Command name selected from the dropdown (drives commandName on submit). */
  selectedCommand: string | null
}

export function useCommandAutocomplete({
  onValueChange,
  commands,
  enabled = true,
}: UseCommandAutocompleteOptions) {
  const [state, setState] = useState<CommandAutocompleteState>({
    isOpen: false,
    searchTerm: '',
    selectedIndex: 0,
    selectedCommand: null,
  })

  const inputRef = useRef<HTMLTextAreaElement>(null)

  // Filter commands client-side based on the current search term
  const filteredCommands = useMemo(() => {
    if (!state.isOpen || commands.length === 0) {
      return []
    }
    if (!state.searchTerm) {
      return commands
    }
    const term = state.searchTerm.toLowerCase()
    return commands.filter(
      (cmd) =>
        cmd.name.toLowerCase().includes(term) || cmd.description.toLowerCase().includes(term),
    )
  }, [state.isOpen, state.searchTerm, commands])

  const handleInputChange = useCallback(
    (newValue: string, _cursorPosition: number) => {
      if (!enabled || commands.length === 0) {
        return
      }

      // Only activate when `/` is the very first character
      if (newValue.length === 0 || newValue[0] !== '/') {
        // If selectedCommand was set but user removed the prefix, clear it
        if (state.selectedCommand) {
          setState((prev) => ({ ...prev, selectedCommand: null, isOpen: false, searchTerm: '' }))
        } else if (state.isOpen) {
          setState((prev) => ({ ...prev, isOpen: false, searchTerm: '', selectedIndex: 0 }))
        }
        return
      }

      // If a command was already selected, check if the prefix still matches
      if (state.selectedCommand) {
        const expectedPrefix = `/${state.selectedCommand} `
        if (!newValue.startsWith(expectedPrefix)) {
          // User edited the command portion — clear selection and re-open autocomplete
          setState((prev) => ({ ...prev, selectedCommand: null }))
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
        selectedCommand: prev.selectedCommand,
        selectedIndex: prev.searchTerm !== searchTerm ? 0 : prev.selectedIndex,
      }))
    },
    [enabled, commands.length, state.selectedCommand, state.isOpen],
  )

  const selectCommand = useCallback(
    (command: CommandResponse) => {
      const newValue = `/${command.name} `
      onValueChange(newValue)

      setState({
        isOpen: false,
        searchTerm: '',
        selectedIndex: 0,
        selectedCommand: command.name,
      })

      // Move cursor to end
      setTimeout(() => {
        if (inputRef.current) {
          const pos = newValue.length
          inputRef.current.setSelectionRange(pos, pos)
          inputRef.current.focus()
        }
      }, 0)
    },
    [onValueChange],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!(state.isOpen && filteredCommands.length > 0)) {
        return
      }

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setState((prev) => ({
            ...prev,
            selectedIndex: Math.min(prev.selectedIndex + 1, filteredCommands.length - 1),
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
          if (filteredCommands[state.selectedIndex]) {
            e.preventDefault()
            selectCommand(filteredCommands[state.selectedIndex])
          }
          break
        case 'Tab':
          if (filteredCommands[state.selectedIndex]) {
            e.preventDefault()
            selectCommand(filteredCommands[state.selectedIndex])
          }
          break
        case 'Escape':
          e.preventDefault()
          e.stopPropagation()
          setState((prev) => ({ ...prev, isOpen: false }))
          break
        default:
          break
      }
    },
    [state.isOpen, state.selectedIndex, filteredCommands, selectCommand],
  )

  /** The full CommandResponse for the currently selected command, if any. */
  const selectedCommandData = useMemo(
    () =>
      state.selectedCommand
        ? (commands.find((c) => c.name === state.selectedCommand) ?? null)
        : null,
    [state.selectedCommand, commands],
  )

  /** Clear the selected command (call on successful submit). */
  const clearSelectedCommand = useCallback(() => {
    setState((prev) => ({ ...prev, selectedCommand: null }))
  }, [])

  return {
    inputRef,
    isOpen: state.isOpen,
    filteredCommands,
    selectedIndex: state.selectedIndex,
    selectedCommand: selectedCommandData,
    handleInputChange,
    handleKeyDown,
    selectCommand,
    clearSelectedCommand,
  }
}
