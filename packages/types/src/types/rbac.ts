/**
 * User roles in the system.
 * Start with simple admin/user, can extend later.
 */
export type UserRole = 'admin' | 'user'

/**
 * Resource types that can be protected.
 * Extensible for future granular permissions.
 */
export type Resource =
  | 'task'
  | 'repository'
  | 'agent'
  | 'preview'
  | 'file'
  | 'settings'
  | 'user'
  | 'system'

/**
 * Actions that can be performed on resources.
 */
export type Action = 'create' | 'read' | 'update' | 'delete' | 'execute' | 'manage' // Full control

/**
 * Permission check request - abstracts role vs permission check.
 * Used internally by PermissionService.
 */
export interface PermissionCheck {
  action: Action
  resource: Resource
  resourceId?: string // Optional for resource-specific checks
}
