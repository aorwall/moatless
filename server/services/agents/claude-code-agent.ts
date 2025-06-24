import type { Event } from '@shared/types/event.js'
import { type ChildProcess, spawn } from 'child_process'
import path from 'path'
import { createInterface, type Interface } from 'readline'
import { type AgentOptions, type AgentRunOptions, BaseAgent } from './base-agent.js'

export interface ClaudeCodeOptions extends AgentOptions {
  cliPath?: string
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  systemPrompt?: string
  tools?: string[]
  outputFormat?: 'json' | 'text'
}

export class ClaudeCodeAgent extends BaseAgent {
  private process?: ChildProcess
  private readline?: Interface
  private cliPath: string

  constructor(taskId: string, options: ClaudeCodeOptions = {}, cliPath?: string) {
    super(taskId, options)
    this.cliPath = cliPath || this.findCli()
  }

  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false

    try {
      yield this.createEvent('start', {
        message,
        agentType: 'claude-code',
        options: this.options,
      })

      const args = this.buildCliArgs(message)

      // Spawn claude process
      this.process = spawn(this.cliPath, args, {
        cwd: runOptions.workspaceDir || process.cwd(),
        env: this.getEnvironment(runOptions.environmentVariables),
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: false,
      })

      // Handle process errors
      this.process.on('error', (error) => {
        console.error('Process error:', error)
      })

      // Create readline interface for stdout
      this.readline = createInterface({
        input: this.process.stdout!,
        crlfDelay: Infinity,
      })

      // Handle stderr
      let stderrOutput = ''
      this.process.stderr?.on('data', (data) => {
        stderrOutput += data.toString()
      })

      // Process each line of output
      let lastEvent: Event | null = null

      for await (const line of this.readline) {
        if (this.stopRequested) {
          yield this.createEvent('stop', { reason: 'user_requested' })
          break
        }

        try {
          // Try to parse as JSON (Claude Code outputs structured events)
          const data = JSON.parse(line)
          const event = this.parseEventFromJson(data)
          lastEvent = event
          yield event
        } catch (e) {
          // Non-JSON output - treat as message content
          if (line.trim()) {
            const event = this.createEvent('message', {
              content: line,
              role: 'assistant',
            })
            lastEvent = event
            yield event
          }
        }
      }

      // Wait for process to exit
      const exitCode = await this.waitForProcessExit()

      if (exitCode === 0) {
        yield this.createEvent('completion', {
          exitCode,
          successful: true,
          lastEvent: lastEvent?.payload,
        })
      } else {
        yield this.createEvent(
          'error',
          {
            exitCode,
            error: `Process exited with code ${exitCode}`,
            stderr: stderrOutput,
          },
          true,
        )
      }
    } catch (error) {
      yield this.createEvent(
        'error',
        {
          error: error instanceof Error ? error.message : String(error),
          stack: error instanceof Error ? error.stack : undefined,
        },
        true,
      )
    } finally {
      await this.cleanup()
    }
  }

  async stop(): Promise<void> {
    this.stopRequested = true

    if (this.process && !this.process.killed) {
      // Send SIGTERM first
      this.process.kill('SIGTERM')

      // Give it 5 seconds to gracefully shutdown
      const timeout = new Promise<void>((resolve) => {
        setTimeout(() => {
          if (this.process && !this.process.killed) {
            this.process.kill('SIGKILL')
          }
          resolve()
        }, 5000)
      })

      const processExit = new Promise<void>((resolve) => {
        if (this.process) {
          this.process.on('close', () => resolve())
        } else {
          resolve()
        }
      })

      await Promise.race([timeout, processExit])
    }

    await this.cleanup()
  }

  private findCli(): string {
    // Port logic from _find_cli method
    const possiblePaths = [
      process.env.CLAUDE_CLI_PATH,
      path.join(process.env.HOME || '', '.npm-global/bin/claude'),
      path.join(process.env.HOME || '', '.local/bin/claude'),
      '/usr/local/bin/claude',
      '/opt/homebrew/bin/claude',
    ].filter(Boolean) as string[]

    // Check which path exists
    for (const cliPath of possiblePaths) {
      try {
        const fs = require('fs')
        fs.accessSync(cliPath, fs.constants.F_OK)
        return cliPath
      } catch {}
    }

    // Default to PATH lookup
    return 'claude'
  }

  private buildCliArgs(message: string): string[] {
    const args: string[] = []
    const options = this.options as ClaudeCodeOptions

    // Add model if specified
    if (options.model) {
      args.push('--model', options.model)
    }

    // Add API key if specified
    if (options.apiKey) {
      args.push('--api-key', options.apiKey)
    }

    // Add max tokens
    if (options.maxTokens) {
      args.push('--max-tokens', String(options.maxTokens))
    }

    // Add temperature
    if (options.temperature !== undefined) {
      args.push('--temperature', String(options.temperature))
    }

    // Add system prompt
    if (options.systemPrompt) {
      args.push('--system', options.systemPrompt)
    }

    // Add tools
    if (options.tools && options.tools.length > 0) {
      for (const tool of options.tools) {
        args.push('--tool', tool)
      }
    }

    // Add output format
    if (options.outputFormat) {
      args.push('--output', options.outputFormat)
    }

    // Add the message
    args.push(message)

    return args
  }

  private getEnvironment(additionalEnv?: Record<string, string>): NodeJS.ProcessEnv {
    const options = this.options as ClaudeCodeOptions

    return {
      ...process.env,
      ...additionalEnv,
      CLAUDE_API_KEY: options.apiKey || process.env.CLAUDE_API_KEY,
      NO_COLOR: '1', // Disable color output for easier parsing
      FORCE_COLOR: '0',
      TERM: 'dumb', // Ensure no terminal-specific behavior
    }
  }

  private parseEventFromJson(data: any): Event {
    // Transform claude CLI output to our event format
    const eventType = this.mapClaudeEventType(data.type || 'message')

    return this.createEvent(
      eventType,
      {
        ...data,
        originalType: data.type,
        content: data.content || data.message || data.text,
        tool: data.tool,
        toolInput: data.tool_input,
        toolOutput: data.tool_output,
      },
      data.error || false,
      data.subtype || data.sub_type,
    )
  }

  private mapClaudeEventType(claudeType: string): string {
    // Map Claude CLI event types to our internal event types
    const mapping: Record<string, string> = {
      text: 'message',
      tool_use: 'tool',
      tool_result: 'tool',
      error: 'error',
      completion: 'completion',
      input: 'input',
      thinking: 'message',
    }

    return mapping[claudeType] || 'message'
  }

  private async waitForProcessExit(): Promise<number> {
    return new Promise((resolve, reject) => {
      if (!this.process) {
        resolve(0)
        return
      }

      this.process.on('close', (code) => {
        resolve(code || 0)
      })

      this.process.on('error', (error) => {
        reject(error)
      })

      // Timeout after configured time or 5 minutes
      const timeout = (this.options.timeout || 300) * 1000
      setTimeout(() => {
        if (this.process && !this.process.killed) {
          this.process.kill('SIGKILL')
          reject(new Error('Process timeout'))
        }
      }, timeout)
    })
  }

  private async cleanup(): Promise<void> {
    if (this.readline) {
      this.readline.close()
      this.readline = undefined
    }

    if (this.process && !this.process.killed) {
      this.process.kill('SIGKILL')
    }

    this.process = undefined
  }
}
