import type { ServiceIdentifier } from 'inversify'
import type { AgentController } from './services/agent-controller.service'
import type { McpSettingsService } from './services/mcp-settings.service'

/**
 * DI symbol for AgentController.
 * Used to break circular module loading issues when injecting AgentController.
 */
export const AgentControllerSymbol: ServiceIdentifier<AgentController> =
  Symbol.for('AgentController')

/**
 * DI symbol for McpSettingsService.
 */
export const McpSettingsServiceSymbol: ServiceIdentifier<McpSettingsService> =
  Symbol.for('McpSettingsService')
