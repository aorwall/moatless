export interface BaseAgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  systemPrompt?: string
}

export interface ClaudeCodeOptions extends BaseAgentOptions {
  cliPath?: string
  tools?: string[]
  outputFormat?: 'json' | 'text'
  workspaceIntegration?: boolean
  autoApprove?: boolean
  maxIterations?: number
}

export interface OpenAIAgentOptions extends BaseAgentOptions {
  organization?: string
  functions?: any[]
  functionCall?: 'auto' | 'none' | { name: string }
}

export class AgentOptionsBuilder {
  private options: Record<string, any> = {}

  model(model: string): this {
    this.options.model = model
    return this
  }

  apiKey(apiKey: string): this {
    this.options.apiKey = apiKey
    return this
  }

  maxTokens(maxTokens: number): this {
    this.options.maxTokens = maxTokens
    return this
  }

  temperature(temperature: number): this {
    this.options.temperature = temperature
    return this
  }

  timeout(timeout: number): this {
    this.options.timeout = timeout
    return this
  }

  systemPrompt(prompt: string): this {
    this.options.systemPrompt = prompt
    return this
  }

  tools(tools: string[]): this {
    this.options.tools = tools
    return this
  }

  outputFormat(format: 'json' | 'text'): this {
    this.options.outputFormat = format
    return this
  }

  build<T = BaseAgentOptions>(): T {
    return { ...this.options } as T
  }
}

export const defaultClaudeCodeOptions: ClaudeCodeOptions = {
  model: 'claude-3-5-sonnet-20241022',
  maxTokens: 8192,
  temperature: 0.1,
  timeout: 300, // 5 minutes
  tools: ['computer', 'bash', 'str_replace_editor'],
  outputFormat: 'json',
  workspaceIntegration: true,
  autoApprove: false,
  maxIterations: 10,
}

export function createClaudeCodeOptions(
  overrides: Partial<ClaudeCodeOptions> = {},
): ClaudeCodeOptions {
  return {
    ...defaultClaudeCodeOptions,
    ...overrides,
  }
}

export function validateAgentOptions(options: BaseAgentOptions): string[] {
  const errors: string[] = []

  if (options.maxTokens !== undefined && (options.maxTokens < 1 || options.maxTokens > 200000)) {
    errors.push('maxTokens must be between 1 and 200000')
  }

  if (options.temperature !== undefined && (options.temperature < 0 || options.temperature > 2)) {
    errors.push('temperature must be between 0 and 2')
  }

  if (options.timeout !== undefined && options.timeout < 1) {
    errors.push('timeout must be at least 1 second')
  }

  return errors
}
