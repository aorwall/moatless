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
  taskInputs: ReturnType<typeof useTaskInputs>
  selectedModel: ClaudeModel
  selectedAgentMode: 'edit' | 'plan'
  setSelectedAgentMode: (mode: 'edit' | 'plan') => void
  isModelSelectorEnabled: boolean
  startFresh: boolean
  setStartFresh: (value: boolean) => void
  resumeConversation: boolean
  pendingUserInput: PendingUserInput | null
}

/**
 * Message-send orchestration for the chat: free-text submission (skills are
 * invoked by typing `/skill` in the message, which the agent resolves from
 * `.claude/skills/`), and pending-tool responses (question answers, plan
 * approval).
 */
export function useChatSend({
  taskId,
  message,
  setMessage,
  taskInputs,
  selectedModel,
  selectedAgentMode,
  setSelectedAgentMode,
  isModelSelectorEnabled,
  startFresh,
  setStartFresh,
  resumeConversation,
  pendingUserInput,
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

  return {
    sendMessage,
    handleChatSubmit,
    handleQuestionSubmit,
    handlePlanApprove,
    handlePlanReject,
  }
}
