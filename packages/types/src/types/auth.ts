/**
 * Authentication provider identifier
 * Can be any string (e.g., 'github', 'auth0', 'google', 'okta')
 * This allows the system to support any OAuth provider without code changes
 */
export type AuthProvider = string

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

// Note: GitHub service connection types are now defined in schemas/settings/github.schemas.ts
// and exported via the Zod schemas. Import them from @moatless/types instead.
