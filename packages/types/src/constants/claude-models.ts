// Available Claude models
export const CLAUDE_MODELS = {
  OPUS_4: 'claude-sonnet-4-5-20250929',
  SONNET_4: 'claude-sonnet-4-5-20250929',
} as const

export type ClaudeModel = (typeof CLAUDE_MODELS)[keyof typeof CLAUDE_MODELS]

// Model display names
export const CLAUDE_MODEL_DISPLAY_NAMES: Record<ClaudeModel, string> = {
  'claude-sonnet-4-5-20250929': 'Sonnet 4.5',
}

// Default model
export const DEFAULT_CLAUDE_MODEL = CLAUDE_MODELS.OPUS_4
