import { apiClient } from '@/lib/api/client'

export type FeedbackSurface = 'task' | 'message' | 'tool_call' | 'preview_failure'
export type FeedbackSource = 'user' | 'agent'

export interface CreateFeedbackRequest {
  taskId: string
  messageUuid?: string
  toolUseId?: string
  surface: FeedbackSurface
  comment: string
  metadata?: Record<string, unknown>
}

export interface FeedbackResponse {
  feedbackId: string
  taskId: string
  messageUuid?: string
  toolUseId?: string
  source: FeedbackSource
  surface: FeedbackSurface
  comment: string
  metadata?: Record<string, unknown>
  createdBy: string
  createdAt: string
}

export interface ListFeedbackResponse {
  items: FeedbackResponse[]
}

export const feedbackApi = {
  submit(req: CreateFeedbackRequest): Promise<FeedbackResponse> {
    return apiClient.post<FeedbackResponse>('/api/v1/feedback', req)
  },

  listForTask(taskId: string): Promise<ListFeedbackResponse> {
    return apiClient.get<ListFeedbackResponse>(`/api/v1/tasks/${taskId}/feedback`)
  },
}
