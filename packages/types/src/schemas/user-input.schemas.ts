import { z } from 'zod'

// === AskUserQuestion Types ===
export const QuestionOptionSchema = z.object({
  label: z.string(),
  description: z.string().optional(),
})

export const QuestionSchema = z.object({
  question: z.string(),
  header: z.string().optional(),
  options: z.array(QuestionOptionSchema),
  multiSelect: z.boolean().default(false),
})

export const AskUserQuestionInputSchema = z.object({
  questions: z.array(QuestionSchema),
})

// === ExitPlanMode Types ===
export const AllowedPromptSchema = z.object({
  tool: z.literal('Bash'),
  prompt: z.string(),
})

export const ExitPlanModeInputSchema = z.object({
  allowedPrompts: z.array(AllowedPromptSchema).optional(),
  plan: z.string().optional(),
})

// === Response Types ===
export const AskUserQuestionResponseSchema = z.object({
  answers: z.record(z.string(), z.string()),
})

export const ExitPlanModeResponseSchema = z.object({
  approved: z.boolean(),
  feedback: z.string().optional(),
})

// Request to submit user input (frontend -> backend -> sandbox)
export const UserInputResponseRequestSchema = z.object({
  toolName: z.enum(['AskUserQuestion', 'ExitPlanMode']),
  response: z.union([AskUserQuestionResponseSchema, ExitPlanModeResponseSchema]),
  toolUseId: z.string().optional(),
})

// Type exports
export type QuestionOption = z.infer<typeof QuestionOptionSchema>
export type Question = z.infer<typeof QuestionSchema>
export type AskUserQuestionInput = z.infer<typeof AskUserQuestionInputSchema>
export type AllowedPrompt = z.infer<typeof AllowedPromptSchema>
export type ExitPlanModeInput = z.infer<typeof ExitPlanModeInputSchema>
export type AskUserQuestionResponse = z.infer<typeof AskUserQuestionResponseSchema>
export type ExitPlanModeResponse = z.infer<typeof ExitPlanModeResponseSchema>
export type UserInputResponseRequest = z.infer<typeof UserInputResponseRequestSchema>
