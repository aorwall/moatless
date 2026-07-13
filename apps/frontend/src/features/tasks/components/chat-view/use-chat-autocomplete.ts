import type React from 'react'
import { useCallback } from 'react'
import {
  type Suggestion,
  useCommandAutocomplete,
} from '@/features/tasks/components/chat/command-autocomplete'
import { useFileAutocomplete } from '@/features/tasks/components/chat/file-autocomplete'
import { useFileSearch } from '@/features/tasks/hooks/use-file-search'

interface UseChatAutocompleteOptions {
  taskId: string
  message: string
  setMessage: (message: string) => void
  /** Merged command + skill suggestions for the "/" dropdown. */
  suggestions: Suggestion[]
  /** Command autocomplete is disabled while the agent is running. */
  commandsEnabled: boolean
}

/**
 * Composes the "@" file autocomplete and the "/" command autocomplete over the
 * chat textarea: a shared input-change handler that feeds both from the cursor
 * position, and a combined key handler (commands take precedence).
 */
export function useChatAutocomplete({
  taskId,
  message,
  setMessage,
  suggestions,
  commandsEnabled,
}: UseChatAutocompleteOptions) {
  // File search mutation (task-specific)
  const fileSearchMutation = useFileSearch(taskId)

  // File autocomplete hook (generic)
  const {
    inputRef: textareaRef,
    isOpen: isFileAutocompleteOpen,
    results: fileResults,
    selectedIndex: fileSelectedIndex,
    isLoading: isFileAutocompleteLoading,
    error: fileAutocompleteError,
    handleInputChange,
    handleKeyDown: handleFileKeyDown,
    selectFile,
  } = useFileAutocomplete({
    value: message,
    onValueChange: setMessage,
    enabled: true,
    onSearch: fileSearchMutation.mutate,
    results: fileSearchMutation.data?.files ?? [],
    isLoading: fileSearchMutation.isPending,
    error: fileSearchMutation.error,
  })

  // Command autocomplete hook (client-side filtering, no extra API call)
  const {
    isOpen: isCommandAutocompleteOpen,
    filteredCommands,
    selectedIndex: commandSelectedIndex,
    selectedCommand,
    handleInputChange: handleCommandInputChange,
    handleKeyDown: handleCommandKeyDown,
    selectCommand,
    clearSelectedCommand,
  } = useCommandAutocomplete({
    onValueChange: setMessage,
    suggestions,
    enabled: commandsEnabled,
  })

  // Handle message changes with autocomplete
  const handleMessageChange = useCallback(
    (newMessage: string) => {
      setMessage(newMessage)
      // Update autocomplete state based on cursor position
      if (textareaRef.current) {
        const cursorPosition = textareaRef.current.selectionStart || 0
        handleCommandInputChange(newMessage, cursorPosition)
        handleInputChange(newMessage, cursorPosition)
      }
    },
    [handleCommandInputChange, handleInputChange, textareaRef, setMessage],
  )

  // Compose key handlers: command autocomplete first, then file autocomplete
  const handleCombinedKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      handleCommandKeyDown(e)
      if (!e.defaultPrevented) {
        handleFileKeyDown(e)
      }
    },
    [handleCommandKeyDown, handleFileKeyDown],
  )

  return {
    textareaRef,
    handleMessageChange,
    handleCombinedKeyDown,
    selectedCommand,
    clearSelectedCommand,
    fileAutocomplete: {
      isOpen: isFileAutocompleteOpen,
      results: fileResults,
      selectedIndex: fileSelectedIndex,
      isLoading: isFileAutocompleteLoading,
      error: fileAutocompleteError,
      selectFile,
    },
    commandAutocomplete: {
      isOpen: isCommandAutocompleteOpen,
      filteredCommands,
      selectedIndex: commandSelectedIndex,
      selectCommand,
    },
  }
}
