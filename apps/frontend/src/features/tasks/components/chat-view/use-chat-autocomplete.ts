import type React from 'react'
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
  /** Skill suggestions for the "/" dropdown. */
  suggestions: Suggestion[]
  /** Skill autocomplete is disabled while the agent is running. */
  commandsEnabled: boolean
  /** Activate a skill picked from the "/" dropdown. */
  onSelectSkill: (name: string) => void
}

/**
 * Composes the "@" file autocomplete and the "/" skill autocomplete over the
 * chat textarea: a shared input-change handler that feeds both from the cursor
 * position, and a combined key handler (skills take precedence).
 */
export function useChatAutocomplete({
  taskId,
  message,
  setMessage,
  suggestions,
  commandsEnabled,
  onSelectSkill,
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

  // Skill autocomplete hook (client-side filtering, no extra API call)
  const {
    isOpen: isCommandAutocompleteOpen,
    filteredCommands,
    selectedIndex: commandSelectedIndex,
    handleInputChange: handleCommandInputChange,
    handleKeyDown: handleCommandKeyDown,
    selectCommand,
  } = useCommandAutocomplete({
    value: message,
    onValueChange: setMessage,
    onSelectSkill,
    suggestions,
    enabled: commandsEnabled,
  })

  // Handle message changes with autocomplete
  const handleMessageChange = (newMessage: string) => {
    setMessage(newMessage)
    // Update autocomplete state based on cursor position
    if (textareaRef.current) {
      const cursorPosition = textareaRef.current.selectionStart || 0
      handleCommandInputChange(newMessage, cursorPosition)
      handleInputChange(newMessage, cursorPosition)
    }
  }

  // Compose key handlers: skill autocomplete first, then file autocomplete
  const handleCombinedKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    handleCommandKeyDown(e)
    if (!e.defaultPrevented) {
      handleFileKeyDown(e)
    }
  }

  return {
    textareaRef,
    handleMessageChange,
    handleCombinedKeyDown,
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
