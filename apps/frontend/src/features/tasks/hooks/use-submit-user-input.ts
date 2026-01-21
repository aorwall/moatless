import type { UserInputResponseRequest } from '@moatless/types'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { tasksApi } from '@/features/tasks/api/tasks'
import { queryKeys } from '@/lib/api/query-keys'

interface UserInputError extends Error {
  canResume?: boolean
  agentSessionId?: string
}

/**
 * Create a resumable error with session ID for recovery
 */
function createResumeError(canResume: boolean, agentSessionId: string): UserInputError {
  const error = new Error(
    'Agent session was interrupted. Please send a message to resume.',
  ) as UserInputError
  error.canResume = canResume
  error.agentSessionId = agentSessionId
  return error
}

/**
 * Check if error is a 409 with resume info and transform it
 */
async function handle409Error(error: unknown): Promise<never> {
  if (!(error instanceof Error && 'response' in error)) {
    throw error
  }

  const response = (error as Error & { response?: Response }).response
  if (response?.status !== 409) {
    throw error
  }

  const data = await response.json().catch(() => null)
  if (data?.canResume && data?.agentSessionId) {
    throw createResumeError(data.canResume, data.agentSessionId)
  }
  throw error
}

/**
 * Hook for submitting user input responses for AskUserQuestion and ExitPlanMode tools.
 * The agent pauses waiting for user input and continues after this response is submitted.
 *
 * If the sandbox has lost state (409 response), the error will include:
 * - canResume: true if the session can be resumed
 * - agentSessionId: the session ID to use for resuming
 */
export function useSubmitUserInput(taskId: string) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationKey: ['tasks', taskId, 'userInputResponse'],
    mutationFn: async (request: UserInputResponseRequest) => {
      try {
        return await tasksApi.submitUserInputResponse(taskId, request)
      } catch (error) {
        await handle409Error(error)
      }
    },
    onSuccess: () => {
      // Invalidate messages to refresh the UI after submission
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.messages(taskId),
      })
    },
    onError: (error: UserInputError) => {
      if (error.canResume && error.agentSessionId) {
        toast.error(
          'The agent session was interrupted. Send a message with "Resume" enabled to continue.',
          { duration: 5000 },
        )
      } else {
        toast.error(error.message || 'Failed to submit response')
      }
    },
  })
}
