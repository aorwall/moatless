export const CLAUDE_MODELS = {
  DEFAULT: 'default',
  OPUS: 'opus',
  SONNET: 'sonnet',
  HAIKU: 'haiku',
  FABLE: 'fable',
} as const

export type ClaudeModel = (typeof CLAUDE_MODELS)[keyof typeof CLAUDE_MODELS]

export const CLAUDE_MODEL_DISPLAY_NAMES: Record<ClaudeModel, string> = {
  default: 'Default',
  opus: 'Claude Opus',
  sonnet: 'Claude Sonnet',
  haiku: 'Claude Haiku',
  fable: 'Claude Fable',
}

export const DEFAULT_CLAUDE_MODEL: ClaudeModel = CLAUDE_MODELS.DEFAULT

/** Convert ClaudeModel to API value. 'default' sentinel becomes undefined (omit from request). */
export function modelToApiValue(model: ClaudeModel): string | undefined {
  return model === CLAUDE_MODELS.DEFAULT ? undefined : model
}
