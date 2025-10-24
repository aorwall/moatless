/**
 * Generic OAuth tokens returned by any provider
 * Providers may include additional provider-specific tokens
 */
export interface OAuthTokens {
  accessToken: string
  tokenType: string
  scope?: string | undefined
  idToken?: string | undefined // Optional (e.g., Auth0, OpenID Connect providers)
  [key: string]: unknown // Provider-specific additional data
}

/**
 * Generic user information structure returned by any provider
 * Normalized format that all providers must map to
 */
export interface ProviderUserInfo {
  id: string
  login: string
  email: string | null
  name: string | null
  avatarUrl: string | null
}
