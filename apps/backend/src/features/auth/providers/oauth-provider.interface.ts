import type { ServiceIdentifier } from 'inversify'
import type { OAuthTokens, ProviderUserInfo } from '../types/oauth.types.js'

/**
 * DI symbol for OAuth Providers Map.
 * Use this symbol to inject the map of OAuth provider implementations.
 */
export const OAuthProvidersSymbol: ServiceIdentifier<Map<string, OAuthProvider>> =
  Symbol.for('OAuthProviders')

/**
 * Generic OAuth Provider Interface
 *
 * This interface is completely implementation-agnostic and knows nothing about
 * specific providers (GitHub, Auth0, Google, etc.). Each provider implementation
 * identifies itself and implements the required methods.
 *
 * Optional methods allow providers to support features like group extraction
 * or custom logout flows without forcing all providers to implement them.
 */
export interface OAuthProvider {
  /**
   * Unique identifier for this provider (e.g., "github", "auth0", "google")
   * Used for session tracking and provider selection
   */
  getProviderId(): string

  /**
   * Generate the OAuth authorization URL for this provider
   * @param state - CSRF protection state token
   * @returns Authorization URL or null if configuration is invalid
   */
  generateAuthUrl(state: string): string | null

  /**
   * Exchange OAuth authorization code for access tokens
   * @param code - Authorization code from OAuth callback
   * @returns OAuth tokens (access token, optional ID token, etc.)
   */
  exchangeCodeForTokens(code: string): Promise<OAuthTokens>

  /**
   * Get authenticated user information using the provided tokens
   * @param tokens - OAuth tokens from successful authentication
   * @returns Normalized user information
   */
  getUserInfo(tokens: OAuthTokens): Promise<ProviderUserInfo>

  /**
   * Optional: Extract user's group/organization from tokens
   * Only implement if the provider supports group-based access control
   * @param tokens - OAuth tokens that may contain group information
   * @returns Group identifier or undefined if not supported/not found
   */
  extractUserGroup?(tokens: OAuthTokens): Promise<string | undefined>

  /**
   * Optional: Generate provider logout URL
   * Only implement if the provider requires server-side logout (e.g., Auth0)
   * @param returnTo - URL to redirect to after logout
   * @returns Logout URL or null if not supported/configuration invalid
   */
  generateLogoutUrl?(returnTo: string): string | null
}
