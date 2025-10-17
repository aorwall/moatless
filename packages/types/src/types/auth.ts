/**
 * Authentication provider types
 */
export type AuthProvider = 'github' | 'auth0'

/**
 * User information from GitHub OAuth
 */
export interface GitHubOAuthUser {
  id: number
  login: string
  email: string | null
  name: string | null
  avatar_url: string | null
}

/**
 * User information from Auth0
 */
export interface Auth0User {
  sub: string // Auth0 user ID (e.g., "auth0|123456")
  email: string | null
  name: string | null
  nickname: string | null
  picture: string | null
  email_verified?: boolean | undefined
  // Allow custom claims (namespaced properties like "https://moatless.ai/groups")
  [key: string]: unknown
}

/**
 * Persisted user data
 */
export interface User {
  id: string // Changed to string to support both GitHub (numeric) and Auth0 (string) IDs
  login: string
  email: string | null
  name: string | null
  avatarUrl: string | null
  provider: AuthProvider
  createdAt: Date
  lastLoginAt: Date
}

/**
 * User session data
 */
export interface UserSession {
  id: string
  provider: AuthProvider
  userId: string // Changed to string to support both GitHub and Auth0 user IDs
  userLogin: string
  userEmail: string | null
  userName: string | null
  avatarUrl: string | null
  accessToken?: string | undefined // Optional for Auth0 (ID token used instead)
  idToken?: string | undefined // Auth0 ID token
  tokenType?: string | undefined
  scope?: string | undefined
  groupId?: string | undefined // Optional group ID for group-based access control
  createdAt: Date
  expiresAt: Date
  lastAccessedAt: Date
}

/**
 * Session response for API
 */
export interface SessionResponse {
  user: {
    id: string
    login: string
    email: string | null
    name: string | null
    avatarUrl: string | null
    provider: AuthProvider
  } | null
  expiresAt: string | null
}

/**
 * GitHub OAuth token response
 */
export interface GitHubOAuthTokenResponse {
  access_token: string
  token_type: string
  scope: string
}

/**
 * Auth0 OAuth token response
 */
export interface Auth0TokenResponse {
  access_token: string
  id_token: string
  token_type: string
  expires_in: number
  scope?: string | undefined
}
