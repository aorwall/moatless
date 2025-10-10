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
 * Persisted user data
 */
export interface User {
  id: number
  login: string
  email: string | null
  name: string | null
  avatarUrl: string | null
  createdAt: Date
  lastLoginAt: Date
}

/**
 * User session data
 */
export interface UserSession {
  id: string
  userId: number
  userLogin: string
  userEmail: string | null
  userName: string | null
  avatarUrl: string | null
  accessToken: string
  tokenType: string
  scope: string
  groupId?: string // Optional group ID for group-based access control
  createdAt: Date
  expiresAt: Date
  lastAccessedAt: Date
}

/**
 * Session response for API
 */
export interface SessionResponse {
  user: {
    id: number
    login: string
    email: string | null
    name: string | null
    avatarUrl: string | null
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
