/**
 * Frontend-specific response types for user input tools.
 *
 * Request/display types (Question, AllowedPrompt, etc.) come from the api-client.
 * These response types are only used by the frontend to submit responses back.
 */

// Re-export display types from api-client for convenience
export type {
  AllowedPrompt,
  AskUserQuestionInput,
  ExitPlanModeInput,
  Question,
  QuestionOption,
} from '@moatless/api-client/sandbox/model'

export interface AskUserQuestionResponse {
  answers: Record<string, string>
}

export interface ExitPlanModeResponse {
  approved: boolean
  feedback?: string
}

export interface UserInputResponseRequest {
  toolName: 'AskUserQuestion' | 'ExitPlanMode'
  response: AskUserQuestionResponse | ExitPlanModeResponse
  toolUseId?: string
}
