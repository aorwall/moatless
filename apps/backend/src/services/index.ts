/**
 * Services Directory
 *
 * This directory contains cross-feature infrastructure services that are used across
 * multiple features and cannot be easily moved to a single feature folder without
 * creating circular dependencies.
 *
 * INFRASTRUCTURE SERVICES (Keep Here):
 * - SettingsService: Application-wide settings management used by agents, workspace,
 *   repositories, and other features
 * - DockerService: Reserved for future Docker-based worker execution support
 *
 * MIGRATED SERVICES (Moved to Feature Folders):
 * - FileOperations → features/workspace/services/
 * - ScreenshotOrchestrationService → features/preview/services/
 */

// Re-export feature services for backward compatibility
export * from '../features/agents/index'
export * from '../features/deployments/index'
export * from '../features/events/index'
export * from '../features/file-storage/index'
export * from '../features/preview/index'
export * from '../features/repositories/index'
export { TaskService } from '../features/tasks/services/task.service'
export * from './docker.service'
export * from './redis-lock.service'
// Cross-feature infrastructure services
export * from './settings.service'
