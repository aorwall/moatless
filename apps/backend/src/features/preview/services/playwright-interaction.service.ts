import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { RepositoryService } from '../../repositories/index.js'
import { PlaywrightUnavailableError } from '../errors.js'
import type { PreviewServerLifecycleSink } from '../types/index.js'
import type {
  ClickResult,
  ConsoleLogsResult,
  EvaluationResult,
  FormFillResult,
  NavigationResult,
  NetworkRequestsResult,
  ScreenshotResult,
  SnapshotResult,
  WaitResult,
} from '../types/playwright-results.js'
import type { PreviewOrchestratorService } from './preview-orchestrator.service.js'

export type PlaywrightMode = 'idle' | 'user' | 'agent'

const UNAVAILABLE_MESSAGE =
  'Playwright is not available in the app container. Browser interactions are handled by the sandbox.'

/**
 * Stub service for Playwright browser interactions.
 * Playwright is not available in the app container - all browser interactions
 * are delegated to the sandbox. This service preserves the API contract but
 * throws PlaywrightUnavailableError for all operations.
 */
@injectable()
export class PlaywrightInteractionService {
  private serviceLogger = logger.child({ service: 'playwright-interaction' })
  private availabilityChecked = false

  /**
   * Initialize the service by marking Playwright as unavailable.
   */
  initialize(): void {
    if (this.availabilityChecked) {
      return
    }
    this.availabilityChecked = true
    this.serviceLogger.info('Playwright disabled - browser interactions delegated to sandbox')
  }

  /**
   * Check if Playwright is available for browser interactions.
   * @returns Always returns false - Playwright is not available in the app container
   */
  isAvailable(): boolean {
    return false
  }

  /**
   * Get the reason why Playwright is unavailable.
   */
  getUnavailableReason(): string {
    return UNAVAILABLE_MESSAGE
  }

  /**
   * Returns a rejected Promise with PlaywrightUnavailableError.
   * Used by all async methods to maintain Promise return types.
   */
  private rejectUnavailable<T>(): Promise<T> {
    return Promise.reject(new PlaywrightUnavailableError(UNAVAILABLE_MESSAGE))
  }

  // Configuration setters (no-op but preserve API)
  setLifecycleSink(_sink: PreviewServerLifecycleSink): void {
    // No-op - API stub
  }
  setPreviewOrchestrator(_orchestrator: PreviewOrchestratorService): void {
    // No-op - API stub
  }
  setRepositoryService(_repositoryService: RepositoryService): void {
    // No-op - API stub
  }

  // All browser interaction methods return rejected Promises

  ensurePlaywrightPage(_taskId: string, _initialUrl: string): Promise<never> {
    return this.rejectUnavailable()
  }

  navigateTo(_taskId: string, _path: string, _serverUrl: string): Promise<NavigationResult> {
    return this.rejectUnavailable()
  }

  clickElement(_taskId: string, _selector: string): Promise<ClickResult> {
    return this.rejectUnavailable()
  }

  evaluateJavaScript(_taskId: string, _script: string): Promise<EvaluationResult> {
    return this.rejectUnavailable()
  }

  fillForm(_taskId: string, _fields: Record<string, string>): Promise<FormFillResult> {
    return this.rejectUnavailable()
  }

  takeSnapshot(_taskId: string): Promise<SnapshotResult> {
    return this.rejectUnavailable()
  }

  takeScreenshot(_taskId: string): Promise<ScreenshotResult> {
    return this.rejectUnavailable()
  }

  takeSnapshotWithAutoRestart(_taskId: string): Promise<SnapshotResult> {
    return this.rejectUnavailable()
  }

  takeScreenshotWithAutoRestart(_taskId: string): Promise<ScreenshotResult> {
    return this.rejectUnavailable()
  }

  typeText(
    _taskId: string,
    _selector: string,
    _text: string,
    _options?: { delay?: number; clear?: boolean },
  ): Promise<NavigationResult> {
    return this.rejectUnavailable()
  }

  hoverElement(_taskId: string, _selector: string): Promise<NavigationResult> {
    return this.rejectUnavailable()
  }

  getConsoleLogs(_taskId: string): Promise<ConsoleLogsResult> {
    return this.rejectUnavailable()
  }

  getNetworkRequests(_taskId: string, _urlPattern?: string): Promise<NetworkRequestsResult> {
    return this.rejectUnavailable()
  }

  waitForElement(
    _taskId: string,
    _selector: string,
    _options?: { state?: 'attached' | 'detached' | 'visible' | 'hidden'; timeout?: number },
  ): Promise<WaitResult> {
    return this.rejectUnavailable()
  }

  setMode(_taskId: string, _mode: PlaywrightMode): Promise<void> {
    return Promise.resolve() // No-op - no page state to track
  }

  getMode(_taskId: string): PlaywrightMode {
    return 'idle'
  }

  getCurrentUrl(_taskId: string): string | undefined {
    return undefined
  }

  closePlaywrightPage(_taskId: string): Promise<void> {
    return Promise.resolve() // No-op - no page to close
  }

  cleanup(): Promise<void> {
    return Promise.resolve() // No-op - no resources to clean up
  }
}
