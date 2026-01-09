import type { Action, Resource, UserRole } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { UserRepository } from '../db/user.repository'
import { ForbiddenError } from '../utils/session-helpers'
import { SessionContext } from './session-context'

/**
 * User role permission matrix.
 * Defines what actions each role can perform on resources.
 * Admins have full access; users have resource-specific permissions.
 */
const USER_PERMISSIONS: Record<Resource, Action[]> = {
  task: ['create', 'read', 'update', 'delete', 'execute'],
  repository: ['create', 'read', 'update', 'delete'],
  agent: ['create', 'read', 'execute'],
  preview: ['create', 'read', 'execute'],
  file: ['create', 'read', 'update', 'delete'],
  settings: ['read'], // Users can only read (their own) settings
  user: ['read'], // Users can only read their own profile
  system: [], // No system access for users
}

/**
 * Centralized permission service for role-based access control.
 *
 * Design goals:
 * 1. Single abstraction for all permission checks
 * 2. Works with roles now, can add granular permissions later
 * 3. No changes to call sites when upgrading to permissions
 *
 * Extension point: When adding granular permissions, update
 * `checkUserPermissions()` to query a permissions table first,
 * then fall back to role defaults.
 */
@injectable()
export class PermissionService {
  constructor(@inject(UserRepository) private readonly userRepo: UserRepository) {}

  /**
   * Check if current user can perform action on resource.
   *
   * Current implementation: Role-based check
   * Future: Can add granular permission lookup
   *
   * @param action - The action to perform
   * @param resource - The resource type
   * @param _resourceId - Optional resource ID (for future per-resource permissions)
   * @returns true if allowed, false otherwise
   */
  can(action: Action, resource: Resource, _resourceId?: string): boolean {
    const session = SessionContext.getCurrentSession()

    // In dev mode (no auth), check environment-based admin override
    if (!session) {
      const isDevMode = process.env['AUTH_PROVIDER'] === 'disabled'
      return isDevMode
    }

    const role = session.role

    // Admin has full access to everything
    if (role === 'admin') {
      return true
    }

    // Check environment-based admin override (for bootstrapping)
    if (this.isEnvAdmin(session.userId)) {
      return true
    }

    // User role permissions
    return this.checkUserPermissions(action, resource)
  }

  /**
   * Assert permission, throwing ForbiddenError if denied.
   * Use this in routes/services for cleaner error handling.
   *
   * @param action - The action to perform
   * @param resource - The resource type
   * @param resourceId - Optional resource ID
   * @throws ForbiddenError if permission denied
   */
  assertCan(action: Action, resource: Resource, resourceId?: string): void {
    const allowed = this.can(action, resource, resourceId)
    if (!allowed) {
      throw new ForbiddenError(
        `Permission denied: cannot ${action} ${resource}${resourceId ? ` (${resourceId})` : ''}`,
      )
    }
  }

  /**
   * Check if current user is admin.
   * Convenience method for admin-only routes.
   */
  isAdmin(): boolean {
    const session = SessionContext.getCurrentSession()
    if (!session) {
      return process.env['AUTH_PROVIDER'] === 'disabled'
    }

    // Check role first
    if (session.role === 'admin') {
      return true
    }

    // Check env-based admin override (for bootstrapping)
    return this.isEnvAdmin(session.userId)
  }

  /**
   * Assert current user is admin.
   * @throws ForbiddenError if not admin
   */
  assertAdmin(): void {
    if (!this.isAdmin()) {
      throw new ForbiddenError('Admin access required')
    }
  }

  /**
   * Get the current user's role.
   * Returns 'user' as default if no session.
   */
  getCurrentRole(): UserRole {
    const session = SessionContext.getCurrentSession()
    if (!session) {
      return 'user'
    }
    return session.role
  }

  /**
   * Get role for a user ID.
   * Used during session creation to cache role.
   *
   * @param userId - User ID to get role for
   * @returns User role or 'user' as default
   */
  async getUserRole(userId: string): Promise<UserRole> {
    const user = await this.userRepo.getUser(userId)
    return user?.role ?? 'user'
  }

  /**
   * Check if user ID is in environment-based admin list.
   * Used for bootstrapping first admin user.
   */
  private isEnvAdmin(userId: string): boolean {
    const envAdmins = process.env['ADMIN_USER_IDS']?.split(',').map((id) => id.trim()) ?? []
    return envAdmins.includes(userId)
  }

  /**
   * User role permission matrix check.
   * Central place to define what 'user' role can do.
   *
   * This is the extension point: when adding granular permissions,
   * this method will first check the permission table, then fall back
   * to role defaults.
   */
  private checkUserPermissions(action: Action, resource: Resource): boolean {
    return USER_PERMISSIONS[resource]?.includes(action) ?? false
  }
}
