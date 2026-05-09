import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  type CreateFeedbackRequest,
  type FeedbackResponse,
  feedbackApi,
} from '@/features/feedback/api/feedback.api'
import { queryKeys } from '@/lib/api/query-keys'

export function useSubmitFeedback() {
  const queryClient = useQueryClient()
  return useMutation<FeedbackResponse, Error, CreateFeedbackRequest>({
    mutationFn: (req) => feedbackApi.submit(req),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.feedback.byTask(variables.taskId) })
    },
  })
}
