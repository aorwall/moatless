import type { ServiceIdentifier } from 'inversify'
import type { ClaudeService } from './services/claude.service'
import type { ClaudeSettingsService } from './services/claude-settings.service'

/**
 * DI symbol for ClaudeService.
 * Used to break circular dependencies when injecting ClaudeService into legacy services.
 */
export const ClaudeServiceSymbol: ServiceIdentifier<ClaudeService> = Symbol.for('ClaudeService')

/**
 * DI symbol for ClaudeSettingsService.
 */
export const ClaudeSettingsServiceSymbol: ServiceIdentifier<ClaudeSettingsService> =
  Symbol.for('ClaudeSettingsService')
