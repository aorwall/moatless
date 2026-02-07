import { spawn as nodeSpawn } from 'node:child_process'
import { access, readFile } from 'node:fs/promises'
import path from 'node:path'
import {
  createSdkMcpServer,
  type SpawnedProcess,
  type SpawnOptions,
  tool as sdkTool,
} from '@anthropic-ai/claude-agent-sdk'
import {
  SANDBOX_CLAUDE_EXECUTABLE_PATH,
  SANDBOX_PLUGINS_DIR,
  SANDBOX_UPLOAD_DIR,
} from '@moatless/sandbox-schemas'
import type { ClaudeCodeOptions } from '@moatless/types'
import { getAgentConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import type { SandboxTool } from '../../tools/types'
import type { AgentOptions, AgentRunRequest } from '../base/base-agent'
import { emitPendingResponseEvent, handleInteractivePrompt } from './interactive-prompt'

const MCP_SERVER_NAME = 'moatless-tools'

/**
 * Context required to build SDK options.
 */
export interface SDKBuildContext {
  request: AgentRunRequest
  workspaceDir: string
  options: AgentOptions
  taskId: string
  sessionId: string | undefined
  agentType: string
  stderrCollector: (data: string) => void
  processExitCollector: (info: { code: number | null; signal: string | null }) => void
  emitEvent: (event: import('@moatless/types').AgentEvent) => void
}

/**
 * Check if a path exists, with a timeout to avoid blocking on stale mounts.
 * Returns false if the path doesn't exist or the check times out.
 */
async function pathExistsWithTimeout(p: string, timeoutMs = 5000): Promise<boolean> {
  try {
    await Promise.race([
      access(p),
      new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new Error(`pathExists timeout after ${timeoutMs}ms: ${p}`)),
          timeoutMs,
        ),
      ),
    ])
    return true
  } catch {
    return false
  }
}

/**
 * Build the full ClaudeCodeOptions for the SDK query call.
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Diagnostic logging increased complexity slightly; refactoring planned
export async function buildSDKOptions(ctx: SDKBuildContext): Promise<ClaudeCodeOptions> {
  const options = ctx.options as ClaudeCodeOptions

  // Plan mode behavior is controlled by system prompt, not SDK permissions
  // FIXME SDK 'plan' permission mode forces ExitPlanMode for every action, causing loops
  const permissionMode = 'acceptEdits'

  // Convert generic SandboxTool[] to MCP server config if tools are provided
  const sandboxTools = (ctx.options as AgentOptions).sandboxTools
  let mcpFromTools: ReturnType<typeof buildMcpFromTools> | undefined
  if (sandboxTools && sandboxTools.length > 0) {
    mcpFromTools = buildMcpFromTools(sandboxTools)
  }

  // Log CLI version from build-time file (avoids execSync timeout in sandbox containers)
  const versionFile = '/opt/moatless/.claude-cli-version'
  let cliVersion = 'unknown'
  try {
    if (await pathExistsWithTimeout(versionFile)) {
      cliVersion = (await readFile(versionFile, 'utf-8')).trim()
    }
  } catch {
    // Ignore read errors — version is informational only
  }
  logger.info({ cliVersion, SANDBOX_CLAUDE_EXECUTABLE_PATH }, 'Claude Code version')

  // Strip non-SDK fields from options before spreading into SDK options
  const { sandboxTools: _sandboxTools, ...sdkCompatibleOptions } = options as AgentOptions &
    Record<string, unknown>
  const sdkOptions: ClaudeCodeOptions = {
    permissionMode,
    ...sdkCompatibleOptions,
    cwd: ctx.workspaceDir || options.cwd || process.cwd(),
    additionalDirectories: [SANDBOX_UPLOAD_DIR],
    settingSources: ['project', 'user'],
    pathToClaudeCodeExecutable: SANDBOX_CLAUDE_EXECUTABLE_PATH,
    ...(mcpFromTools && { mcpServers: { [mcpFromTools.serverName]: mcpFromTools.server } }),
    stderr: (data: string) => {
      ctx.stderrCollector(data)
      // Log at WARN level so it's visible even without DEBUG
      logger.warn({ stderr: data }, 'Claude Code stderr output')
    },
    spawnClaudeCodeProcess: (spawnOptions: SpawnOptions): SpawnedProcess => {
      logger.info(
        {
          command: spawnOptions.command,
          args: spawnOptions.args,
          cwd: spawnOptions.cwd,
          envKeys: Object.keys(spawnOptions.env),
        },
        'Spawning Claude Code process',
      )

      const child = nodeSpawn(spawnOptions.command, spawnOptions.args, {
        cwd: spawnOptions.cwd,
        env: spawnOptions.env as NodeJS.ProcessEnv,
        signal: spawnOptions.signal,
        stdio: ['pipe', 'pipe', 'pipe'],
      })

      let exited = false

      child.on('exit', (code, signal) => {
        exited = true
        ctx.processExitCollector({
          code,
          signal: signal as string | null,
        })
        logger.info({ exitCode: code, signal, pid: child.pid }, 'Claude Code process exited')
      })

      child.on('error', (err) => {
        logger.error({ err, pid: child.pid }, 'Claude Code process error')
      })

      // Escalate to SIGKILL if process doesn't exit after abort (SIGTERM).
      // This handles processes stuck in D state (uninterruptible I/O wait)
      // where SIGTERM is ignored.
      if (spawnOptions.signal) {
        spawnOptions.signal.addEventListener(
          'abort',
          () => {
            setTimeout(() => {
              if (!exited && child.pid) {
                logger.warn(
                  { pid: child.pid },
                  'Claude Code process did not exit after abort, sending SIGKILL',
                )
                try {
                  process.kill(child.pid, 'SIGKILL')
                } catch {
                  // Process may have already exited
                }
              }
            }, 5_000)
          },
          { once: true },
        )
      }

      // Pipe stderr to our collector + logger
      if (child.stderr) {
        child.stderr.on('data', (chunk: Buffer) => {
          const line = chunk.toString()
          ctx.stderrCollector(line)
          logger.warn({ stderr: line }, 'Claude Code stderr (from spawn)')
        })
      }

      return child as unknown as SpawnedProcess
    },
    // Handle user input tools (AskUserQuestion, ExitPlanMode)
    canUseTool: async (toolName, toolInput) => {
      if (toolName === 'AskUserQuestion' || toolName === 'ExitPlanMode') {
        // Emit pending_response event for channel notification BEFORE waiting
        emitPendingResponseEvent(toolName, toolInput, {
          taskId: ctx.taskId,
          agentType: ctx.agentType,
          sessionId: ctx.sessionId,
          emitEvent: ctx.emitEvent,
        })
        return await handleInteractivePrompt(toolName, toolInput, ctx.taskId)
      }
      // Allow all tools
      return { behavior: 'allow', updatedInput: toolInput }
    },
  }

  if (sdkOptions.mcpServers) {
    logger.info({ mcpServerNames: Object.keys(sdkOptions.mcpServers) }, 'MCP servers configured')
  }

  // Get agent config from REPO_CONFIG (static, per-task settings)
  // This includes: plugins, timeout, model (default), systemPrompt
  const sandboxAgentConfig = getAgentConfig()

  // Add model if provided (prioritize: request.model > agentConfig.model > options.model)
  if (ctx.request.model) {
    sdkOptions.model = ctx.request.model
  } else if (sandboxAgentConfig.model) {
    sdkOptions.model = sandboxAgentConfig.model
  } else if (options.model) {
    sdkOptions.model = options.model
  }

  // Only pass string session IDs to SDK (boolean true should have been resolved by consumer)
  if (typeof ctx.request.resume === 'string') {
    sdkOptions.resume = ctx.request.resume
  }

  // Configure plugins from REPO_CONFIG.agentConfig (static, per-task)
  // Prefer pre-resolved pluginPaths from backend over legacy plugin ID resolution
  if (sandboxAgentConfig.pluginPaths && sandboxAgentConfig.pluginPaths.length > 0) {
    // Use pre-resolved relative paths from backend PluginService
    const resolvedPaths = sandboxAgentConfig.pluginPaths.map((relativePath) =>
      path.join(SANDBOX_PLUGINS_DIR, relativePath),
    )
    const validPlugins: { type: 'local'; path: string }[] = []
    for (const p of resolvedPaths) {
      if (await pathExistsWithTimeout(p)) {
        validPlugins.push({ type: 'local' as const, path: p })
      } else {
        logger.error(
          { pluginPath: p },
          'Plugin directory not found on filesystem (or mount timed out) - plugin will not be loaded',
        )
      }
    }
    sdkOptions.plugins = validPlugins

    logger.info(
      {
        configuredPluginPaths: sandboxAgentConfig.pluginPaths.length,
        resolvedPaths: resolvedPaths.length,
        activePlugins: sdkOptions.plugins.length,
        pluginPaths: sdkOptions.plugins.map((p) => p.path),
      },
      'Plugins configured from pre-resolved pluginPaths',
    )
  } else if (sandboxAgentConfig.plugins && sandboxAgentConfig.plugins.length > 0) {
    // Fallback: legacy resolution from plugin IDs (backward compatibility)
    const resolvedPaths = resolvePluginPaths(sandboxAgentConfig.plugins)
    const validPlugins: { type: 'local'; path: string }[] = []
    for (const p of resolvedPaths) {
      if (await pathExistsWithTimeout(p)) {
        validPlugins.push({ type: 'local' as const, path: p })
      } else {
        logger.error(
          { pluginPath: p },
          'Plugin directory not found on filesystem (or mount timed out) - plugin will not be loaded',
        )
      }
    }
    sdkOptions.plugins = validPlugins

    logger.info(
      {
        configuredPlugins: sandboxAgentConfig.plugins.length,
        resolvedPaths: resolvedPaths.length,
        activePlugins: sdkOptions.plugins.length,
        pluginPaths: sdkOptions.plugins.map((p) => p.path),
      },
      'Plugins configured from REPO_CONFIG.agentConfig (legacy resolution)',
    )
  } else {
    logger.debug('No plugins configured in REPO_CONFIG.agentConfig')
  }

  // Use the system prompt from REPO_CONFIG (static, per-task)
  if (sandboxAgentConfig.systemPrompt) {
    sdkOptions.systemPrompt = {
      type: 'preset',
      preset: 'claude_code',
      append: sandboxAgentConfig.systemPrompt,
    }
  }

  // Append per-request system prompt from backend (e.g., preview server instructions)
  if (ctx.request.appendSystemPrompt) {
    if (
      sdkOptions.systemPrompt &&
      typeof sdkOptions.systemPrompt === 'object' &&
      sdkOptions.systemPrompt.append
    ) {
      sdkOptions.systemPrompt.append += `\n\n${ctx.request.appendSystemPrompt}`
    } else {
      sdkOptions.systemPrompt = {
        type: 'preset',
        preset: 'claude_code',
        append: ctx.request.appendSystemPrompt,
      }
    }
    logger.debug(
      { appendLength: ctx.request.appendSystemPrompt.length },
      'Per-request system prompt appended',
    )
  }

  return sdkOptions
}

/**
 * Resolve plugin identifiers to sandbox-absolute paths.
 * Plugin ID format: {plugin-name}@{marketplace-name}
 * Resolves to: /mnt/plugins/marketplaces/{marketplace-name}/{plugin-name}
 */
function resolvePluginPaths(pluginIds: string[]): string[] {
  return pluginIds
    .map((pluginId) => {
      const atIndex = pluginId.lastIndexOf('@')
      if (atIndex === -1) {
        logger.warn({ pluginId }, 'Invalid plugin ID format (missing @)')
        return null
      }
      const pluginName = pluginId.substring(0, atIndex)
      const marketplaceName = pluginId.substring(atIndex + 1)
      return path.join(SANDBOX_PLUGINS_DIR, 'marketplaces', marketplaceName, pluginName)
    })
    .filter((p): p is string => p !== null)
}

/**
 * Convert SandboxTool[] to an MCP server config for the SDK.
 */
function buildMcpFromTools(tools: SandboxTool[]) {
  const sdkTools = tools.map((t) =>
    sdkTool(t.name, t.description, t.schema, async (args) => {
      const result = await t.execute(args as Record<string, unknown>)
      return {
        content: [{ type: 'text' as const, text: result.text }],
        ...(result.isError && { isError: true }),
      }
    }),
  )

  const server = createSdkMcpServer({
    name: MCP_SERVER_NAME,
    version: '1.0.0',
    tools: sdkTools,
  })

  const toolNames = tools.map((t) => `mcp__${MCP_SERVER_NAME}__${t.name}`)

  return { server, serverName: MCP_SERVER_NAME, toolNames }
}
