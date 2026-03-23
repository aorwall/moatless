export const CLAUDE_MODELS = {
  DEFAULT: 'default',
  OPUS_4: 'claude-opus-4-6',
  SONNET_4: 'claude-sonnet-4-6',
  HAIKU_4: 'claude-haiku-4-5-20251001',
} as const

export type ClaudeModel = (typeof CLAUDE_MODELS)[keyof typeof CLAUDE_MODELS]

export const CLAUDE_MODEL_DISPLAY_NAMES: Record<ClaudeModel, string> = {
  default: 'Default',
  'claude-opus-4-6': 'Opus 4.6',
  'claude-sonnet-4-6': 'Sonnet 4.6',
  'claude-haiku-4-5-20251001': 'Haiku 4.5',
}

export const DEFAULT_CLAUDE_MODEL: ClaudeModel = CLAUDE_MODELS.DEFAULT

/** Convert ClaudeModel to API value. 'default' sentinel becomes undefined (omit from request). */
export function modelToApiValue(model: ClaudeModel): string | undefined {
  return model === CLAUDE_MODELS.DEFAULT ? undefined : model
}
