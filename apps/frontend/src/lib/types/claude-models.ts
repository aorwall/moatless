export const CLAUDE_MODELS = {
  DEFAULT: 'default',
  OPUS: 'claude-opus-4-8',
  SONNET: 'claude-sonnet-5',
  HAIKU: 'claude-haiku-4-5',
  FABLE: 'claude-fable-5',
} as const

export type ClaudeModel = (typeof CLAUDE_MODELS)[keyof typeof CLAUDE_MODELS]

export const CLAUDE_MODEL_DISPLAY_NAMES: Record<ClaudeModel, string> = {
  default: 'Default',
  'claude-opus-4-8': 'Opus 4.8',
  'claude-sonnet-5': 'Sonnet 5',
  'claude-haiku-4-5': 'Haiku 4.5',
  'claude-fable-5': 'Fable 5',
}

export const DEFAULT_CLAUDE_MODEL: ClaudeModel = CLAUDE_MODELS.DEFAULT

/** Convert ClaudeModel to API value. 'default' sentinel becomes undefined (omit from request). */
export function modelToApiValue(model: ClaudeModel): string | undefined {
  return model === CLAUDE_MODELS.DEFAULT ? undefined : model
}
