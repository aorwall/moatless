import type { ServiceIdentifier } from 'inversify'
import type { ClaudeService } from './implementations/claude-code/claude.service'
import type { AgentController } from './services/agent-controller.service'
import type { ClaudeSettingsService } from './services/claude-settings.service'
import type { McpSettingsService } from './services/mcp-settings.service'

/**
 * DI symbol for ClaudeService.
 * Used to break circular dependencies when injecting ClaudeService into legacy services.
 */
export const ClaudeServiceSymbol: ServiceIdentifier<ClaudeService> = Symbol.for('ClaudeService')

/**
 * DI symbol for AgentController.
 * Used to break circular module loading issues when injecting AgentController.
 */
export const AgentControllerSymbol: ServiceIdentifier<AgentController> =
  Symbol.for('AgentController')

/**
 * DI symbol for ClaudeSettingsService.
 */
export const ClaudeSettingsServiceSymbol: ServiceIdentifier<ClaudeSettingsService> =
  Symbol.for('ClaudeSettingsService')

/**
 * DI symbol for McpSettingsService.
 */
export const McpSettingsServiceSymbol: ServiceIdentifier<McpSettingsService> =
  Symbol.for('McpSettingsService')
