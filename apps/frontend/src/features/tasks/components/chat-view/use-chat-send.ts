import type { CommandResponse } from '@moatless/api-client'
import { toast } from 'sonner'
import type { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useSendMessage } from '@/features/tasks/hooks/use-tasks'
import { type ClaudeModel, modelToApiValue } from '@/lib/types/claude-models'
import { getErrorMessage } from '@/lib/utils/error'
import type { PendingUserInput } from './use-pending-user-input'

interface UseChatSendOptions {
  taskId: string
  message: string
  setMessage: (message: string) => void
  selectedCommand: CommandResponse | null
  clearSelectedCommand: () => void
  taskInputs: ReturnType<typeof useTaskInputs>
  selectedModel: ClaudeModel
  selectedAgentMode: 'edit' | 'plan'
  setSelectedAgentMode: (mode: 'edit' | 'plan') => void
  isModelSelectorEnabled: boolean
  startFresh: boolean
  setStartFresh: (value: boolean) => void
  resumeConversation: boolean
  pendingUserInput: PendingUserInput | null
  availableCommands: CommandResponse[]
}

/** Extract the user text typed after the `/{commandName} ` prefix. */
function extractCommandText(message: string, commandName: string): string {
  const prefix = `/${commandName} `
  return message.startsWith(prefix) ? message.slice(prefix.length).trim() : ''
}

/**
 * Message-send orchestration for the chat: free-text and command submission,
 * command-button clicks, external command execution (preview troubleshoot
 * button), and pending-tool responses (question answers, plan approval).
 */
export function useChatSend({
  taskId,
  message,
  setMessage,
  selectedCommand,
  clearSelectedCommand,
  taskInputs,
  selectedModel,
  selectedAgentMode,
  setSelectedAgentMode,
  isModelSelectorEnabled,
  startFresh,
  setStartFresh,
  resumeConversation,
  pendingUserInput,
  availableCommands,
}: UseChatSendOptions) {
  // Task message sending hook
  const sendMessage = useSendMessage(taskId, {
    onError: (error) => {
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      })
    },
  })

  // Handlers for user input submission (routed through sendMessage)
  const handleQuestionSubmit = (answers: Record<string, string>) => {
    sendMessage.mutate({
      message: '',
      toolUseId: pendingUserInput?.toolUseId,
      toolName: 'AskUserQuestion',
      toolResponse: { answers },
    })
  }

  const handlePlanApprove = () => {
    sendMessage.mutate({
      message: '',
      toolUseId: pendingUserInput?.toolUseId,
      toolName: 'ExitPlanMode',
      toolResponse: { approved: true },
    })
    // Auto-switch to edit mode after approving plan
    setSelectedAgentMode('edit')
  }

  const handlePlanReject = (feedback?: string) => {
    sendMessage.mutate({
      message: '',
      toolUseId: pendingUserInput?.toolUseId,
      toolName: 'ExitPlanMode',
      toolResponse: { approved: false, feedback },
    })
  }

  // Handle sending message
  const handleChatSubmit = () => {
    const trimmedMessage = message.trim()
    if (sendMessage.isPending || !trimmedMessage) {
      return
    }

    // If a command was selected from the autocomplete, send as command
    if (selectedCommand) {
      const commandData = {
        message: extractCommandText(message, selectedCommand.name),
        commandName: selectedCommand.name,
        resume: selectedCommand.willNotResume !== true,
        model: isModelSelectorEnabled ? modelToApiValue(selectedModel) : undefined,
        agentMode: selectedCommand.agentMode || selectedAgentMode,
        fileIds: taskInputs.getFileIds(),
        fileUploads: taskInputs.uploadedFiles,
        contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
        startFresh: startFresh || undefined,
      }

      sendMessage.mutate(commandData, {
        onSuccess: () => {
          setMessage('')
          clearSelectedCommand()
          taskInputs.clearAll()
          setStartFresh(false)
        },
      })
      return
    }

    const messageData = {
      message: trimmedMessage,
      resume: resumeConversation,
      model: isModelSelectorEnabled ? modelToApiValue(selectedModel) : undefined,
      fileIds: taskInputs.getFileIds(),
      fileUploads: taskInputs.uploadedFiles,
      contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
      agentMode: selectedAgentMode,
      startFresh: startFresh || undefined,
    }

    sendMessage.mutate(messageData, {
      onSuccess: () => {
        setMessage('')
        taskInputs.clearAll()
        setStartFresh(false)
      },
    })
  }

  // Handle command button click
  const handleCommandClick = (command: CommandResponse) => {
    const commandData = {
      message: '',
      commandName: command.name,
      resume: command.willNotResume !== true,
      model: isModelSelectorEnabled ? modelToApiValue(selectedModel) : undefined,
      agentMode: command.agentMode || selectedAgentMode,
      fileIds: taskInputs.getFileIds(),
      fileUploads: taskInputs.uploadedFiles,
      contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
    }

    sendMessage.mutate(commandData, {
      onSuccess: () => {
        taskInputs.clearAll()
      },
    })
  }

  // Execute a command by name — used by external callers (e.g. preview troubleshoot button)
  const executeCommandByName = (commandName: string) => {
    const command = availableCommands.find((c) => c.name === commandName)
    if (!command) {
      return
    }

    const commandData = {
      message: '',
      commandName: command.name,
      resume: command.willNotResume !== true,
      model: isModelSelectorEnabled ? modelToApiValue(selectedModel) : undefined,
      agentMode: command.agentMode || selectedAgentMode,
    }
    sendMessage.mutate(commandData)
  }

  return {
    sendMessage,
    handleChatSubmit,
    handleCommandClick,
    executeCommandByName,
    handleQuestionSubmit,
    handlePlanApprove,
    handlePlanReject,
  }
}
