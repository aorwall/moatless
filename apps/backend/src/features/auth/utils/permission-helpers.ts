import type { Action, Resource } from '@moatless/types'
import { appContainer } from '../../../container/container'
import { PermissionService } from '../services/permission.service'

/**
 * Functional helper for permission checks.
 * Use when you need a one-liner without DI.
 *
 * @param action - The action to perform
 * @param resource - The resource type
 * @param resourceId - Optional resource ID
 * @returns true if allowed, false otherwise
 */
export function can(action: Action, resource: Resource, resourceId?: string): boolean {
  const permissionService = appContainer.get(PermissionService)
  return permissionService.can(action, resource, resourceId)
}

/**
 * Functional helper for permission assertion.
 * Throws ForbiddenError if permission denied.
 *
 * @param action - The action to perform
 * @param resource - The resource type
 * @param resourceId - Optional resource ID
 * @throws ForbiddenError if permission denied
 */
export function assertCan(action: Action, resource: Resource, resourceId?: string): void {
  const permissionService = appContainer.get(PermissionService)
  permissionService.assertCan(action, resource, resourceId)
}

/**
 * Check if current user is admin.
 *
 * @returns true if current user is admin
 */
export function isAdmin(): boolean {
  const permissionService = appContainer.get(PermissionService)
  return permissionService.isAdmin()
}

/**
 * Assert current user is admin.
 *
 * @throws ForbiddenError if not admin
 */
export function assertAdmin(): void {
  const permissionService = appContainer.get(PermissionService)
  permissionService.assertAdmin()
}
