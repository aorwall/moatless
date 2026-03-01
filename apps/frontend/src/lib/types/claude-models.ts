export const CLAUDE_MODELS = {
  OPUS_4: 'claude-opus-4-5-20251101',
  SONNET_4: 'claude-sonnet-4-5-20250929',
} as const

export type ClaudeModel = (typeof CLAUDE_MODELS)[keyof typeof CLAUDE_MODELS]

export const CLAUDE_MODEL_DISPLAY_NAMES: Record<ClaudeModel, string> = {
  'claude-opus-4-5-20251101': 'Opus 4.5',
  'claude-sonnet-4-5-20250929': 'Sonnet 4.5',
}

export const DEFAULT_CLAUDE_MODEL = CLAUDE_MODELS.OPUS_4
